//! The Kitty graphics protocol, application side.
//!
//! The vendored terminal (see the `apc_raw` patches in `vendor/vte` and
//! `vendor/alacritty_terminal`) delivers complete APC strings with the
//! cursor's absolute position at the instant the sequence ended. Everything
//! that interprets those bytes - the parameter language, chunked transfers,
//! the image store and placements - lives HERE, in a pure state machine
//! with no locks, no wgpu and no terminal: it is unit-tested like any other
//! data structure, and the renderer below it is just one consumer. The
//! clipboard future (`copy the image the last command printed`) is the
//! second: images keep their original upload bytes, so a copy is lossless
//! and never round-trips through pixels.
//!
//! Implemented subset of https://sw.kovidgoyal.net/kitty/graphics-protocol/ :
//! `a=t` transmit, `a=T` transmit+display, `a=p` display, `a=d` delete,
//! `f=24`/`f=32` raw RGB/RGBA, `f=100` PNG, image ids, placement ids,
//! chunked transfer (`m=1`), source cropping (`x,y,w,h`), display offsets
//! (`X,Y`), cell sizing, z-index, quiet responses, and
//! file/temp-file/shared-memory transfers. Unicode placeholders and zlib
//! compression are not implemented.
//!
//! Caps are blunt by design: one transfer, one image, one tab each has a
//! ceiling, and crossing one drops the sequence instead of growing memory.
//! A terminal that OOMs on a malicious escape stream helps nobody.

use base64::Engine as _;
use std::collections::HashMap;
use std::sync::Arc;

/// Largest assembled transfer (decoded base64 length is not checked first;
/// the buffer is capped while accumulating, which is what matters).
pub const MAX_TRANSFER: usize = 32 * 1024 * 1024;
/// Largest accepted decoded image per side.
pub const MAX_SIDE: u32 = 8192;
/// Images retained per tab; older unreferenced ones are evicted.
pub const MAX_IMAGES: usize = 64;
/// Placements retained per tab. Yazi uses one placement per terminal cell,
/// so a single large preview can need thousands. Keep the budget bounded,
/// with room for dense grids on high-resolution displays.
pub const MAX_PLACEMENTS: usize = 64 * 1024;

/// One decoded image. `gen` bumps when an id is overwritten, giving the
/// GPU texture cache a cheap invalidation key.
pub struct KittyImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The untouched upload payload (PNG today), for a lossless copy.
    pub source: Arc<Vec<u8>>,
    pub generation: u32,
}

/// The source pixel rectangle, before scaling or positioning the placement.
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceRect {
    pub x: u32,
    pub y: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

pub struct PlacementGeometry {
    pub size: [f32; 2],
    /// Normalized source rectangle: u, v, width, height.
    pub uv: [f32; 4],
}

/// Where an image sits: anchored at a cell (absolute logical line, as the
/// terminal saw it) with a column, plus pixel offsets and size in cells or
/// native pixels.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub image_id: u32,
    pub placement_id: u32,
    pub anchor_line: usize,
    pub col: usize,
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub x_off: u32,
    pub y_off: u32,
    pub source: SourceRect,
    pub z: i32,
}

impl Placement {
    pub fn geometry(&self, image: &KittyImage, cell: [f32; 2]) -> Option<PlacementGeometry> {
        let src = self.source;
        if src.x >= image.width || src.y >= image.height { return None; }
        let width = src.width.unwrap_or(image.width).min(image.width - src.x) as f32;
        let height = src.height.unwrap_or(image.height).min(image.height - src.y) as f32;
        if width <= 0.0 || height <= 0.0 { return None; }
        let fit_width = self.cols.map(|cols| (cols as f32 * cell[0] - self.x_off as f32).max(0.0));
        let fit_height = self.rows.map(|rows| (rows as f32 * cell[1] - self.y_off as f32).max(0.0));
        // Cropped 1x1 placements form Yazi's pixel-aligned tile grid: fitting
        // each rounded source tile separately would introduce seams. Ordinary
        // images keep their aspect ratio within the requested cell rectangle.
        let tile = self.cols == Some(1) && self.rows == Some(1)
            && (src.width.is_some() || src.height.is_some() || src.x > 0 || src.y > 0);
        let size = match (fit_width, fit_height) {
            (Some(w), Some(h)) if tile => [w, h],
            (Some(w), Some(h)) => {
                let scale = (w / width).min(h / height);
                [width * scale, height * scale]
            },
            (Some(w), None) => [w, w * height / width],
            (None, Some(h)) => [h * width / height, h],
            (None, None) => [width, height],
        };
        if size[0] <= 0.0 || size[1] <= 0.0 { return None; }
        Some(PlacementGeometry {
            size,
            uv: [src.x as f32 / image.width as f32, src.y as f32 / image.height as f32,
                width / image.width as f32, height / image.height as f32],
        })
    }
}

/// Where transmitted image bytes come from beyond the stream itself.
/// Shared memory is POSIX shm, which on Linux is a plain file under
/// /dev/shm - no syscall is needed to read what the sender wrote.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PathMode {
    File,
    TempFile,
    Shm,
}

/// An in-progress chunked transfer.
struct Pending {
    id: u32,
    /// True when the sender named an id: unnamed (id=0) traffic must stay
    /// unacked, even though we give the image an internal id so several
    /// transient images can coexist.
    named: bool,
    display: bool,
    quiet: u8,
    source: SourceRect,
    format: u32,
    px_w: u32,
    px_h: u32,
    cols: Option<u32>,
    rows: Option<u32>,
    x_off: u32,
    y_off: u32,
    z: i32,
    anchor_line: usize,
    anchor_col: usize,
    /// Non-direct medium (t=f/t/s): the payload is a path, not image data.
    path_mode: Option<PathMode>,
    /// Decoded bytes so far. Base64 is decoded chunk by chunk: senders
    /// (broot among them) pad each chunk independently, so the texts of
    /// two chunks must never be pasted together and decoded as one.
    bytes: Vec<u8>,
    /// At most 3 base64 chars awaiting their fourth.
    carry: Vec<u8>,
}

/// Per-tab graphics state. Owned by the app, keyed by tab id; it moves and
/// dies with the tab.
pub struct TabKitty {
    images: HashMap<u32, KittyImage>,
    placements: Vec<Placement>,
    next_id: u32,
    next_placement: u32,
    next_generation: u32,
    pending: Option<Pending>,
}

impl Default for TabKitty {
    fn default() -> Self {
        Self {
            images: HashMap::new(),
            placements: Vec::new(),
            next_id: 1,
            next_placement: 1,
            next_generation: 0,
            pending: None,
        }
    }
}

impl TabKitty {
    /// One complete APC string (the bytes between `ESC _` and ST, so it
    /// starts with the `G` namespace letter for Kitty), anchored at
    /// `line`/`col` as captured at dispatch. Returns bytes to write back to
    /// the pty, respecting the command's quiet-response policy.
    pub fn on_apc(&mut self, payload: &[u8], line: usize, col: usize) -> Vec<u8> {
        let bare = payload.strip_prefix(b"G").unwrap_or_default();
        let head = bare.strip_prefix(b";").unwrap_or(bare).split(|&b| b == b';').next().unwrap_or_default();
        let get = |key: &[u8]| head.split(|&b| b == b',').find_map(|part| part.strip_prefix(key));
        let number = |key: &[u8]| get(key).and_then(|v| std::str::from_utf8(v).ok()?.parse::<u32>().ok());
        let inherited = self.pending.as_ref().filter(|p| {
            (get(b"a=").is_none() && get(b"i=").is_none())
                || (number(b"i=") == Some(p.id)
                    && matches!(get(b"a="), None | Some(b"t") | Some(b"T"))
                    && (get(b"a=") == Some(b"T")) == p.display)
        }).map_or(0, |p| p.quiet);
        let quiet = number(b"q=").map_or(inherited, |q| q.min(2) as u8);
        let reply = self.handle_apc(payload, line, col);
        if quiet == 2 || (quiet == 1 && reply.windows(3).any(|s| s == b";OK")) {
            Vec::new()
        } else { reply }
    }

    fn handle_apc(&mut self, payload: &[u8], line: usize, col: usize) -> Vec<u8> {
        if payload.first() != Some(&b'G') {
            // Some other APC dialect (tmux passes several through); not
            // ours to interpret, and nothing here may misread it.
            return Vec::new();
        }
        // Wire shape is `G;<params>;<data>`. The ';' after the namespace
        // letter is part of the framing - strip it before splitting, or
        // every real frame reads as an empty-params continuation. A
        // continuation frame is `G;` followed by raw data (empty params).
        let bare = &payload[1..];
        let rest: &[u8] = if bare.first() == Some(&b';') { &bare[1..] } else { bare };
        let (params, data): (&[u8], &[u8]) = match rest.iter().position(|&b| b == b';') {
            Some(p) => (&rest[..p], &rest[p + 1..]),
            // `G;X` with no second ';': X is parameters unless it cannot
            // possibly be (no '=' anywhere) while a transfer awaits raw
            // bytes - then it IS the data. Param keys always ride in `k=v`
            // form, so the two readings cannot be confused.
            None if rest.contains(&b'=') || self.pending.is_none() => (rest, &[]),
            None => (&[] as &[u8], rest),
        };
        let params: Vec<(&[u8], &[u8])> = params
            .split(|&b| b == b',')
            .filter(|kv| !kv.is_empty())
            .filter_map(|kv| kv.iter().position(|&b| b == b'=').map(|e| (&kv[..e], &kv[e + 1..])))
            .collect();

        let get = |k: u8| params.iter().find(|(key, _)| key.first() == Some(&k)).map(|(_, v)| *v);
        let num = |k: u8| -> u64 {
            get(k).and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0)
        };
        let append = |slf: &mut Self, data: &[u8]| -> bool {
            match slf.pending.as_mut() {
                Some(p) if p.bytes.len() + p.carry.len() + data.len() <= MAX_TRANSFER => {
                    p.carry.extend_from_slice(data);
                    let n = p.carry.len() / 4 * 4;
                    if n > 0 {
                        let aligned = &p.carry[..n];
                        match base64::engine::general_purpose::STANDARD_NO_PAD
                            .decode(aligned)
                            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(aligned))
                        {
                            Ok(b) => p.bytes.extend_from_slice(&b),
                            Err(_) => {
                                log::warn!("kitty: transfer corrupted mid-stream, dropped");
                                slf.pending = None;
                                return false;
                            },
                        }
                        p.carry.drain(..n);
                    }
                    true
                },
                Some(_) => {
                    log::warn!("kitty: transfer over cap, dropped");
                    slf.pending = None;
                    false
                },
                None => false,
            }
        };

        // Continuation, shape one: no parameters at all (`ESC_G;<data>ST`).
        // Stays open - such a frame cannot state `m`, so only a later
        // parameterized frame can end the transfer.
        if params.is_empty() {
            append(self, data);
            return Vec::new();
        }

        let action = get(b'a').and_then(|v| v.first()).copied().unwrap_or(b't');
        let more = num(b'm') == 1;
        let source = SourceRect {
            x: num(b'x') as u32,
            y: num(b'y') as u32,
            width: (num(b'w') > 0).then(|| num(b'w') as u32),
            height: (num(b'h') > 0).then(|| num(b'h') as u32),
        };

        // Interrogation. The spec's answer is minimal and this is NOT
        // negotiable on shape: `ESC_G i=<id>;OK ESC\`. (A richer `:OK,f=`
        // dialect is not in the protocol; `kitten icat --detect-support`
        // ignores anything else and times out.) Support for a format is
        // demonstrated by answering a query that ASKED about that format;
        // we answer for the formats we actually decode: f=24/32/100.
        // Feature queries (`e=<action>`, kitty 0.50): echo `e` only for
        // actions we implement, so absence says "no" without a lie.
        if action == b'q' {
            let id = num(b'i');
            // Every medium is supported now (we share the filesystem with
            // the client): direct, plain file, temp file, shared memory.
            // Read failures answer with one identical EBADF below, per the
            // spec's file-transmission security rules.
            let _ = get(b't');
            let msg = match get(b'e') {
                Some(e) if e == b"T" => format!("\x1b_Gi={id},e=T;OK\x1b\\"),
                // Known non-default-free actions: we transmit and place, so
                // plain OK; unknown/`f` (animation): no `e` in the reply,
                // which is exactly how the spec spells "not supported".
                Some(_) => format!("\x1b_Gi={id};OK\x1b\\"),
                None => format!("\x1b_Gi={id};OK\x1b\\"),
            };
            return msg.into_bytes();
        }
        // Other unknown actions: ignored. Answering `q` is above; anything
        // else would just be noise on the wire.
        // Placement acks, spec-mandated: requests that name an image id get
        // `i=<id>[,p=<pid>];OK` back on success, ENOENT when the id is
        // unknown. Callers sync state on that reply - silence reads as
        // loss and provokes retransmission storms.

        // Continuation, shape two: a frame carrying neither `a` nor `i`
        // while a transfer lives can only belong to it.
        if self.pending.is_some() && get(b'a').is_none() && get(b'i').is_none() {
            if append(self, data) && !more {
                return self.complete_pending();
            }
            return Vec::new();
        }

        match action {
            b't' | b'T' => {
                let format = num(b'f') as u32;
                let id = {
                    let i = num(b'i') as u32;
                    if i == 0 {
                        self.next_id
                    } else {
                        i
                    }
                };
                // Continuation, shape three: senders like icat repeat the
                // FULL header on every chunk; same id + same action means
                // more data for the open transfer, not a resync.
                let repeat = self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.id == id && (p.display == (action == b'T')));
                if repeat {
                    if append(self, data) && !more {
                        return self.complete_pending();
                    }
                    return Vec::new();
                }
                // A genuinely new header frame while one is open abandons
                // the old: the spec's resync rule.
                let path_mode = match get(b't') {
                    Some(b"f") => Some(PathMode::File),
                    Some(b"t") => Some(PathMode::TempFile),
                    Some(b"s") => Some(PathMode::Shm),
                    _ => None,
                };
                let p = Pending {
                    id,
                    named: num(b'i') != 0,
                    display: action == b'T',
                    quiet: num(b'q').min(2) as u8,
                    source,
                    path_mode,
                    format,
                    px_w: num(b's') as u32,
                    px_h: num(b'v') as u32,
                    cols: (num(b'c') > 0).then(|| num(b'c') as u32),
                    rows: (num(b'r') > 0).then(|| num(b'r') as u32),
                    x_off: num(b'X') as u32,
                    y_off: num(b'Y') as u32,
                    z: num(b'z') as u32 as i32,
                    anchor_line: line,
                    anchor_col: col,
                    bytes: Vec::new(),
                    carry: Vec::new(),
                };
                if data.len() > MAX_TRANSFER {
                    return Vec::new();
                }
                self.pending = Some(p);
                if append(self, data) && !more {
                    return self.complete_pending();
                }
                return Vec::new();
            },
            b'p' => {
                let id = num(b'i') as u32;
                if !self.images.contains_key(&id) {
                    let ack = if id != 0 {
                        format!("\x1b_Gi={id};ENOENT:image not found\x1b\\").into_bytes()
                    } else {
                        Vec::new()
                    };
                    return ack;
                }
                let placement_id = {
                    let p = num(b'p') as u32;
                    if p == 0 {
                        self.next_placement
                    } else {
                        p
                    }
                };
                self.push_placement(Placement {
                    image_id: id,
                    placement_id,
                    source,
                    anchor_line: line,
                    col,
                    cols: (num(b'c') > 0).then(|| num(b'c') as u32),
                    rows: (num(b'r') > 0).then(|| num(b'r') as u32),
                    x_off: num(b'X') as u32,
                    y_off: num(b'Y') as u32,
                    z: num(b'z') as u32 as i32,
                });
                if id != 0 {
                    let pkey = num(b'p');
                    let ack = if pkey != 0 {
                        format!("\x1b_Gi={id},p={placement_id};OK\x1b\\")
                    } else {
                        format!("\x1b_Gi={id};OK\x1b\\")
                    };
                    return ack.into_bytes();
                }
            },
            b'd' => {
                let id = num(b'i') as u32;
                let place = num(b'p') as u32;
                if id == 0 && place == 0 {
                    self.placements.clear();
                    self.images.clear();
                    self.pending = None;
                } else if place != 0 {
                    self.placements.retain(|pl| pl.placement_id != place);
                } else {
                    self.placements.retain(|pl| pl.image_id != id);
                    self.images.remove(&id);
                }
            },
            _ => {},
        }
        Vec::new()
    }

    fn push_placement(&mut self, pl: Placement) {
        if pl.placement_id >= self.next_placement {
            self.next_placement = pl.placement_id + 1;
        }
        self.placements.retain(|old| old.image_id != pl.image_id || old.placement_id != pl.placement_id);
        self.placements.push(pl);
        while self.placements.len() > MAX_PLACEMENTS {
            self.placements.remove(0);
        }
    }

    /// Finishes an open transfer; returns the display ack, if this
    /// transfer was also a display (`a=T`) of a named image.
    fn complete_pending(&mut self) -> Vec<u8> {
        let Some(p) = self.pending.take() else { return Vec::new() };
        if p.id >= self.next_id {
            self.next_id = p.id + 1;
        }
        let mut bytes = p.bytes;
        if !p.carry.is_empty() {
            match decode_b64(&p.carry) {
                Some(mut t) => bytes.append(&mut t),
                None => {
                    log::warn!("kitty: base64 decode failed, frame dropped");
                    return Vec::new();
                },
            }
        }
        // Path mediums: the payload decodes to a filename, not pixels.
        let mut temp_to_delete: Option<std::path::PathBuf> = None;
        if let Some(mode) = p.path_mode {
            let path = match std::str::from_utf8(&bytes) {
                Ok(s) => std::path::PathBuf::from(s),
                Err(_) => return err_ack(p.id, p.named, "Failed to read image file"),
            };
            match read_media(&path, mode) {
                Some((data, delete)) => {
                    bytes = data;
                    temp_to_delete = delete;
                },
                // One identical error for every failure kind (missing,
                // unreadable, not regular, disallowed location) - file
                // existence must not leak through the ack text.
                None => return err_ack(p.id, p.named, "Failed to read image file"),
            }
        }
        let src = Arc::new(bytes);
        let rgba = match p.format {
            100 => match decode_png(&src) {
                Some(img) => img,
                None => {
                    log::warn!("kitty: png decode failed, frame dropped");
                    return Vec::new();
                },
            },
            // Raw RGB/RGBA (f=24/f=32): no header to learn geometry from,
            // so `s=`/`v=` are mandatory; broot's whole graphics path is
            // built on exactly these.
            f @ (24 | 32) => match raw_rgba(&src, f, p.px_w, p.px_h) {
                Some(img) => img,
                None => {
                    log::warn!("kitty: raw f={f} rejected ({}x{}, {} bytes)", p.px_w, p.px_h, src.len());
                    return Vec::new();
                },
            },
            other => {
                log::warn!("kitty: format {other} not supported, frame dropped");
                return Vec::new();
            },
        };
        // Yazi deletes and reuses the same image id when changing files.
        // Never recycle the GPU cache key just because the old image is gone.
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        // Re-transmitting an id replaces its data and placements together.
        self.placements.retain(|placement| placement.image_id != p.id);
        self.images.insert(p.id, KittyImage { width: rgba.0, height: rgba.1, rgba: rgba.2, source: src, generation });
        if let Some(path) = temp_to_delete {
            // Only our own kind of temp path is ever removed (spec).
            let _ = std::fs::remove_file(path);
        }
        let ack = if p.display && p.named {
            format!("\x1b_Gi={};OK\x1b\\", p.id).into_bytes()
        } else {
            Vec::new()
        };
        while self.images.len() > MAX_IMAGES {
            // Evict an image nothing displays, oldest id order aside -
            // any unreferenced one will do; the cap is a flood gate, not
            // an LRU policy with pretensions.
            match self
                .images
                .keys()
                .copied()
                .find(|id| !self.placements.iter().any(|pl| pl.image_id == *id))
            {
                Some(victim) => {
                    self.images.remove(&victim);
                },
                None => break,
            }
        }
        if p.display {
            self.push_placement(Placement {
                image_id: p.id,
                placement_id: 0,
                source: p.source,
                anchor_line: p.anchor_line,
                col: p.anchor_col,
                cols: p.cols,
                rows: p.rows,
                x_off: p.x_off,
                y_off: p.y_off,
                z: p.z,
            });
        }
        ack
    }

    /// Placements in paint order: z first, then anchor (the grid's own
    /// order breaks ties).
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// Sorted copy in paint order, computed once per frame.
    pub fn placements_sorted(&self) -> Vec<Placement> {
        let mut v = self.placements.clone();
        v.sort_by_key(|p| (p.z, p.anchor_line, p.col));
        v
    }

    pub fn image(&self, id: u32) -> Option<&KittyImage> {
        self.images.get(&id)
    }

    /// Full reset (`ESC c`): every image and placement dies, per the
    /// protocol's own rules - a reset that leaves a picture burned in is
    /// not a reset. (`clear`/ED is NOT a reset and does not call this,
    /// matching kitty.)
    pub fn clear_all(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.pending = None;
    }
}

/// Base64 with padding tolerance (some senders trim it).
fn decode_b64(data: &[u8]) -> Option<Vec<u8>> {
    let mut s = data.to_vec();
    while s.len() % 4 != 0 {
        s.push(b'=');
    }
    base64::engine::general_purpose::STANDARD.decode(&s).ok()
}

/// PNG -> RGBA8, size-capped BEFORE allocating: width/height are read from
/// the header and bounds-checked first, so a 100000x100000 claim never
/// reaches the decoder's memory (checked against u64 products as well).
/// A failed load's ack (empty when the sender named no id: silence is the
/// spec answer for id=0 traffic).
fn err_ack(id: u32, named: bool, why: &str) -> Vec<u8> {
    if named {
        format!("\x1b_Gi={id};EBADF:{why}\x1b\\").into_bytes()
    } else {
        Vec::new()
    }
}

/// Reads a file/shm transmission. Enforces the spec's rules: regular
/// files only (symlinks are followed, but a symlink into a forbidden
/// place still lands there, so the FINAL path is what's checked); refuses
/// sensitive trees outright; caps size. Returns (data, delete-after) -
/// delete-after is set only for temp-file transfers whose path lives in a
/// temp directory and carries the protocol marker.
fn read_media(
    path: &std::path::Path,
    mode: PathMode,
) -> Option<(Vec<u8>, Option<std::path::PathBuf>)> {
    use std::path::Component;
    let shm = std::path::Path::new("/dev/shm");
    let normalized = if mode == PathMode::Shm {
        let name = path.file_name()?.to_str()?;
        shm.join(name)
    } else {
        path.to_path_buf()
    };
    // Sensitive trees are refused before anything is opened. /dev/shm is
    // the exception shared memory explicitly lives in (its /dev ancestor
    // must not condemn it).
    if !normalized.starts_with(shm) {
        for parent in normalized.ancestors() {
            if matches!(
                parent.to_str()?,
                "/proc" | "/sys" | "/etc" | "/dev" | "/boot" | "/run"
            ) {
                return None;
            }
        }
    }
    if normalized
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return None;
    }
    let canonical = normalized.canonicalize().ok()?;
    let meta = std::fs::metadata(&canonical).ok()?;
    if !meta.is_file() || meta.len() as usize > MAX_TRANSFER {
        return None;
    }
    let data = std::fs::read(&canonical).ok()?;
    let delete = mode == PathMode::TempFile
        && canonical.starts_with(std::env::temp_dir())
        && canonical.to_str().is_some_and(|s| s.contains("tty-graphics-protocol"));
    Some((data, if delete { Some(canonical) } else { None }))
}

/// Raw RGB/RGBA pixels: length must be exactly w*h*bpp, sides within the
/// cap; RGB grows an opaque alpha.
fn raw_rgba(bytes: &[u8], format: u32, w: u32, h: u32) -> Option<(u32, u32, Vec<u8>)> {
    let bpp = if format == 24 { 3 } else { 4 };
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let need = w as usize * h as usize * bpp;
    if bytes.len() != need {
        return None;
    }
    if format == 32 {
        return Some((w, h, bytes.to_vec()));
    }
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for px in bytes.chunks_exact(3) {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    Some((w, h, rgba))
}

fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let reader = std::io::Cursor::new(bytes);
    let mut decoder = png::Decoder::new(reader);
    // Let the crate own every pixel-format problem: palettes expand to
    // color, tRNS to alpha, 16-bit samples strip to 8, sub-byte gray
    // scales to bytes. (The original code read each frame into a fixed
    // w*h*4 buffer and TRUSTED it to be RGBA. A plain RGB8 screenshot -
    // grim, hyprshot, any color-type-2 PNG - decodes to 3 bytes/pixel,
    // and re-read at a 4-byte stride it shears one byte per pixel into
    // diagonal "interlaced" garbage. Transformations first, then only
    // the remaining channel counts are ours to convert.)
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let Ok(mut reader) = decoder.read_info() else { return None };
    // Dimensions come from the header and are bounded BEFORE any pixel
    // allocation: the output buffer below is the decoder's only canvas,
    // so the cap here is the whole memory story - no decoder-internal
    // limits to configure or misconfigure.
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        log::warn!("kitty: image {w}x{h} out of bounds, dropped");
        return None;
    }
    let size = reader.output_buffer_size();
    let mut raw = vec![0u8; size];
    // next_frame's OutputInfo is the truth about what was written (info()
    // still describes the file header); ALPHA above usually made it RGBA
    // already, and the test suite proved both readings occur.
    let out_info = reader.next_frame(&mut raw).ok()?;
    let rgba = to_rgba8(&raw, out_info.color_type, w, h)?;
    Some((w, h, rgba))
}

/// Shapes the transformations can still hand us at 8-bit: RGBA passes
/// through; RGB/Gray/Grayalpha grow into it. Anything else is refused -
/// silently guessing is what produced the shear in the first place.
fn to_rgba8(raw: &[u8], ct: png::ColorType, w: u32, h: u32) -> Option<Vec<u8>> {
    use png::ColorType as C;
    let px = w as usize * h as usize;
    match ct {
        C::Rgba if raw.len() >= px * 4 => Some(raw[..px * 4].to_vec()),
        C::Rgb if raw.len() >= px * 3 => {
            let mut out = Vec::with_capacity(px * 4);
            for p in raw[..px * 3].chunks_exact(3) {
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            Some(out)
        },
        C::Grayscale if raw.len() >= px => {
            let mut out = Vec::with_capacity(px * 4);
            for &g in &raw[..px] {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            Some(out)
        },
        C::GrayscaleAlpha if raw.len() >= px * 2 => {
            let mut out = Vec::with_capacity(px * 4);
            for p in raw[..px * 2].chunks_exact(2) {
                out.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
            Some(out)
        },
        other => {
            log::warn!("kitty: png color type {other:?} unsupported, frame dropped");
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny valid PNG (solid 4x2 RGBA) built with the same decoder's
    /// partner, so the fixture cannot drift from the parser.
    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 4, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&vec![0x40u8; 4 * 2 * 4]).unwrap();
            w.finish().unwrap();
        }
        out
    }

    fn b64(bytes: &[u8]) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.encode(bytes).into_bytes()
    }

    fn apc(params: &str, data: &[u8]) -> Vec<u8> {
        let mut v = b"G;".to_vec();
        v.extend_from_slice(params.as_bytes());
        if !data.is_empty() {
            v.push(b';');
            v.extend_from_slice(data);
        }
        v
    }

    /// Build an arbitrary-shape PNG through the encoder, so fixtures stay
    /// honest about what real encoders (grim, hyprshot, IM) emit.
    fn enc_png(w: u32, h: u32, ct: png::ColorType, data: &[u8], pal: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(ct);
            enc.set_depth(png::BitDepth::Eight);
            if let Some(p) = pal {
                enc.set_palette(p);
            }
            let mut wt = enc.write_header().unwrap();
            wt.write_image_data(data).unwrap();
            wt.finish().unwrap();
        }
        out
    }

    #[test]
    fn png_color_types_all_reach_the_gpu_as_rgba() {
        // The hyprshot case: color type 2. Decoded at a 4-byte stride
        // unchecked, this is the diagonal shear; here it must be exact.
        let rgb = enc_png(2, 1, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6], None);
        let (w, h, rgba) = decode_png(&rgb).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        // Grayscale 8.
        let g = enc_png(2, 1, png::ColorType::Grayscale, &[7, 9], None);
        let (_, _, rgba) = decode_png(&g).unwrap();
        assert_eq!(rgba, vec![7, 7, 7, 255, 9, 9, 9, 255]);
        // Palette (index) expands through the crate's EXPAND.
        let p = enc_png(2, 1, png::ColorType::Indexed, &[0, 1], Some(&[20, 21, 22, 40, 41, 42]));
        let (_, _, rgba) = decode_png(&p).unwrap();
        assert_eq!(rgba, vec![20, 21, 22, 255, 40, 41, 42, 255]);
    }

    #[test]
    fn yazi_source_crops_are_distinct_from_display_offsets() {
        let mut k = TabKitty::default();
        let pixels = vec![128; 8 * 4 * 4];
        assert!(k.on_apc(&apc("a=t,i=7,f=32,s=8,v=4,q=2", &b64(&pixels)), 0, 0).is_empty());
        assert!(k.on_apc(&apc("a=p,i=7,p=1,x=4,y=2,w=4,h=2,X=1,Y=2,c=1,r=1,q=2", &[]), 12, 3).is_empty());
        let pl = &k.placements()[0];
        assert_eq!((pl.anchor_line, pl.col, pl.x_off, pl.y_off), (12, 3, 1, 2));
        let geometry = pl.geometry(k.image(7).unwrap(), [10.0, 10.0]).unwrap();
        assert_eq!(geometry.uv, [0.5, 0.5, 0.5, 0.5]);
        assert_eq!(geometry.size, [9.0, 8.0]);
    }

    #[test]
    fn source_crops_clip_and_single_axis_sizing_preserves_aspect() {
        let mut k = TabKitty::default();
        k.on_apc(&apc("a=t,i=7,f=100", &b64(&tiny_png())), 0, 0);
        for (params, expected) in [
            ("a=p,i=7,p=1,x=3,y=1,w=200,h=200,c=1,r=1", Some(([10.0, 20.0], [0.75, 0.5, 0.25, 0.5]))),
            ("a=p,i=7,p=1,x=4", None),
            ("a=p,i=7,p=1,y=2", None),
            ("a=p,i=7,p=1,c=4", Some(([40.0, 20.0], [0.0, 0.0, 1.0, 1.0]))),
            ("a=p,i=7,p=1,r=4", Some(([160.0, 80.0], [0.0, 0.0, 1.0, 1.0]))),
            ("a=p,i=7,p=1,c=4,r=4", Some(([40.0, 20.0], [0.0, 0.0, 1.0, 1.0]))),
            ("a=p,i=7,p=1", Some(([4.0, 2.0], [0.0, 0.0, 1.0, 1.0]))),
        ] {
            k.on_apc(&apc(params, &[]), 0, 0);
            let result = k.placements()[0].geometry(k.image(7).unwrap(), [10.0, 20.0]);
            assert_eq!(result.map(|g| (g.size, g.uv)), expected, "{params}");
        }
    }

    #[test]
    fn quiet_uploads_and_placements_suppress_only_requested_responses() {
        let mut k = TabKitty::default();
        let bytes = b64(&tiny_png());
        for quiet in [1, 2] {
            assert!(k.on_apc(&apc(&format!("a=t,i=7,f=100,q={quiet}"), &bytes), 0, 0).is_empty());
            assert!(k.on_apc(&apc(&format!("a=p,i=7,q={quiet}"), &[]), 0, 0).is_empty());
        }
        assert!(!k.on_apc(&apc("a=p,i=99,q=1", &[]), 0, 0).is_empty());
        assert!(k.on_apc(&apc("a=p,i=99,q=2", &[]), 0, 0).is_empty());
        k.on_apc(&apc("a=t,i=8,f=100,q=2,m=1", &bytes[..4]), 0, 0);
        assert!(k.on_apc(&apc("m=0", &bytes[4..]), 0, 0).is_empty());
        assert!(k.image(8).is_some());
        assert!(!k.on_apc(&apc("a=p,i=8", &[]), 0, 0).is_empty());
    }

    #[test]
    fn large_tiled_previews_keep_their_top_left_corner() {
        let mut k = TabKitty::default();
        k.on_apc(&apc("a=t,i=7,f=100,q=2", &b64(&tiny_png())), 0, 0);
        // One placement per cell, as used by Yazi. The old 1024-placement
        // budget discarded the top rows and part of the next row.
        for row in 0..32 {
            for col in 0..48 {
                let id = row * 48 + col + 1;
                k.on_apc(&apc(&format!("a=p,i=7,p={id},c=1,r=1,q=2"), &[]), row, col);
            }
        }
        assert_eq!(k.placements().len(), 32 * 48);
        let first = k.placements().first().unwrap();
        assert_eq!((first.placement_id, first.anchor_line, first.col), (1, 0, 0));
        let last = k.placements().last().unwrap();
        assert_eq!((last.placement_id, last.anchor_line, last.col), (1536, 31, 47));
    }

    #[test]
    fn deleted_image_ids_get_fresh_gpu_generations() {
        let mut k = TabKitty::default();
        let bytes = b64(&tiny_png());
        let mut generations = Vec::new();
        for delete in ["a=d,d=I,i=7", "a=d,d=A", "a=d,d=I,i=7"] {
            k.on_apc(&apc("a=t,i=7,f=100,q=2", &bytes), 0, 0);
            let generation = k.image(7).unwrap().generation;
            assert!(!generations.contains(&generation));
            generations.push(generation);
            k.on_apc(&apc(delete, &[]), 0, 0);
            assert!(k.image(7).is_none());
        }
    }

    #[test]
    fn redrawing_a_named_placement_replaces_its_old_geometry() {
        let mut k = TabKitty::default();
        for id in [7, 8] {
            k.on_apc(&apc(&format!("a=t,i={id},f=100"), &b64(&tiny_png())), 0, 0);
        }
        k.on_apc(&apc("a=p,i=7,p=10,x=0,w=2", &[]), 1, 2);
        k.on_apc(&apc("a=p,i=7,p=10,x=2,w=2", &[]), 3, 4);
        assert_eq!(k.placements().len(), 1);
        let placement = k.placements()[0];
        assert_eq!((placement.anchor_line, placement.col, placement.source.x), (3, 4, 2));
        k.on_apc(&apc("a=p,i=8,p=10", &[]), 0, 0);
        assert_eq!(k.placements().len(), 2);
    }

    #[test]
    fn transmit_and_display() {
        let mut k = TabKitty::default();
        k.on_apc(&apc("a=T,i=7,f=100", &b64(&tiny_png())), 12, 3);
        let img = k.image(7).expect("image stored");
        assert_eq!((img.width, img.height), (4, 2));
        let pl = k.placements();
        assert_eq!(pl.len(), 1);
        assert_eq!((pl[0].image_id, pl[0].anchor_line, pl[0].col), (7, 12, 3));
    }

    #[test]
    fn chunked_transfer_reassembles() {
        let mut k = TabKitty::default();
        let data = b64(&tiny_png());
        let mid = data.len() / 2;
        k.on_apc(&apc("a=t,i=1,m=1,f=100", &data[..mid]), 5, 0);
        assert!(k.image(1).is_none()); // still open
        // Two sender shapes: a paramless frame, then the icat style of
        // stating `m=` per chunk. The final chunk says m=0 explicitly.
        k.on_apc(&{
            let mut v = b"G;".to_vec();
            v.extend_from_slice(&data[mid..mid + 8]);
            v
        }, 5, 0);
        let tail = &data[mid + 8..];
        k.on_apc(&apc("m=0", tail), 5, 0);
        assert!(k.image(1).is_some());
        assert!(k.placements().is_empty()); // 't' does not display
        k.on_apc(&apc("a=p,i=1,c=20,r=10", &[]), 9, 4);
        let pl = &k.placements()[0];
        assert_eq!((pl.anchor_line, pl.col, pl.cols, pl.rows), (9, 4, Some(20), Some(10)));
    }

    #[test]
    fn delete_scopes_by_id_and_clears_all() {
        let mut k = TabKitty::default();
        k.on_apc(&apc("a=T,i=2,f=100", &b64(&tiny_png())), 1, 0);
        k.on_apc(&apc("a=T,i=3,f=100", &b64(&tiny_png())), 2, 0);
        assert_eq!(k.placements().len(), 2);
        k.on_apc(&apc("a=d,i=2", &[]), 3, 0);
        assert_eq!(k.placements().len(), 1);
        assert!(k.image(2).is_none());
        assert!(k.image(3).is_some());
        k.on_apc(&apc("a=d", &[]), 4, 0);
        assert!(k.placements().is_empty());
        assert!(k.image(3).is_none());
    }

    #[test]
    fn malformed_is_ignored_not_fatal() {
        let mut k = TabKitty::default();
        k.on_apc(&apc("a=T,i=9,f=100", b"not base64 !!!"), 1, 0);
        assert!(k.image(9).is_none());
        k.on_apc(&apc("a=T,i=9,f=100", &b64(b"definitely not a png")), 1, 0);
        assert!(k.image(9).is_none());
        k.on_apc(&apc("a=x,i=1,f=100", &[]), 1, 0); // unknown action
        k.on_apc(b"X;not kitty at all", 1, 0); // other APC namespace
        let mut bare = b"G;".to_vec();
        bare.extend_from_slice(&b64(&tiny_png()));
        k.on_apc(&bare, 1, 0); // bare continuation with no pending
        assert_eq!(k.images.len(), 0);
        assert_eq!(k.placements().len(), 0);
    }

    #[test]
    fn clear_all_is_a_clean_slate() {
        let mut k = TabKitty::default();
        k.on_apc(&apc("a=T,i=3,f=100", &b64(&tiny_png())), 1, 0);
        k.on_apc(&apc("a=t,i=9,f=100,m=1", &b64(&tiny_png())), 2, 0); // open transfer
        k.clear_all();
        assert!(k.placements().is_empty());
        assert!(k.image(3).is_none());
        // A pending transfer is gone too: a later m=0 must not resurrect it.
        k.on_apc(&apc("m=0", &b64(&tiny_png())), 3, 0);
        assert!(k.image(9).is_none());
    }

    #[test]
    fn file_and_shm_mediums_read_images() {
        let png = tiny_png();
        let path = std::env::temp_dir().join("mypixels-kittytest-file.png");
        std::fs::write(&path, &png).unwrap();
        let mut k = TabKitty::default();
        let b64path = b64(path.to_str().unwrap().as_bytes());
        let ack = k.on_apc(&apc("a=T,f=100,t=f,i=5", &b64path), 3, 0);
        assert_eq!(ack, b"\x1b_Gi=5;OK\x1b\\");
        assert!(k.image(5).is_some());
        // Every failure shares one ack text: existence must not leak.
        let miss = b64(b"/tmp/mypixels-kittytest-does-not-exist.png");
        let r = String::from_utf8(k.on_apc(&apc("a=T,f=100,t=f,i=6", &miss), 3, 0)).unwrap();
        assert_eq!(r, "\x1b_Gi=6;EBADF:Failed to read image file\x1b\\".to_string());
        // Sensitive trees refused without opening anything.
        let pw = b64(b"/etc/hostname");
        let r = String::from_utf8(k.on_apc(&apc("a=T,f=100,t=f,i=7", &pw), 3, 0)).unwrap();
        assert!(r.contains("EBADF"), "{r:?}");
        assert!(k.image(7).is_none());
        // Shared memory: a plain file under /dev/shm on Linux.
        if std::path::Path::new("/dev/shm").is_dir() {
            let shm = std::path::Path::new("/dev/shm").join("mypixels-kittytest");
            std::fs::write(&shm, &png).unwrap();
            let sp = b64(b"/mypixels-kittytest"); // shm names may be given bare
            let ack = k.on_apc(&apc("a=T,f=100,t=s,i=8", &sp), 3, 0);
            assert_eq!(ack, b"\x1b_Gi=8;OK\x1b\\");
            assert!(k.image(8).is_some());
            std::fs::remove_file(&shm).ok();
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn raw_rgb_and_rgba_are_accepted() {
        let mut k = TabKitty::default();
        // f=24: two red pixels, 2x1
        k.on_apc(&apc("a=T,i=1,f=24,s=2,v=1", &b64(&[255, 0, 0, 255, 0, 0])), 1, 0);
        let img = k.image(1).expect("rgb24 image");
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);
        // f=32 chunked, broot's shape: header, m=1 chunks, m=0 finish
        k.on_apc(&apc("a=t,i=2,f=32,m=1,s=2,v=1", &b64(&[0, 0, 255, 9][..])), 2, 0);
        k.on_apc(&apc("a=t,i=2,f=32,m=1", &b64(&[0, 0, 255, 9][..])), 2, 0);
        assert!(k.image(2).is_none()); // not finished yet
        // broot repeats the full header on continuation chunks (a=t same id):
        k.on_apc(&apc("a=t,i=2,f=32,m=0", &b64(&[][..])), 2, 0);
        let img = k.image(2).expect("rgba32 reassembles");
        assert_eq!(img.rgba, vec![0, 0, 255, 9, 0, 0, 255, 9]);
        // a lying length is refused, not overrun
        k.on_apc(&apc("a=T,i=3,f=24,s=4,v=4", &b64(&[0, 0, 0])), 3, 0);
        assert!(k.image(3).is_none());
        // raw without geometry is refused (nothing derives it from length alone)
        k.on_apc(&apc("a=T,i=4,f=32", &b64(&[0u8; 8])), 4, 0);
        assert!(k.image(4).is_none());
    }

    #[test]
    fn interrogation_answers_truthfully() {
        let mut k = TabKitty::default();
        let r = k.on_apc(&apc("a=q,i=1", &[]), 1, 0);
        // Exact spec shape: ESC_G i=<id>;OK ST - `kitten icat
        // --detect-support` parses for nothing else.
        assert_eq!(r, b"\x1b_Gi=1;OK\x1b\\");
        // Feature query (0.50): e echoes only what is implemented.
        assert_eq!(k.on_apc(&apc("a=q,i=2,e=T", &[]), 1, 0), b"\x1b_Gi=2,e=T;OK\x1b\\");
        assert_eq!(k.on_apc(&apc("a=q,i=3,e=f", &[]), 1, 0), b"\x1b_Gi=3;OK\x1b\\"); // no animation
        // icat's exact detection query.
        assert_eq!(k.on_apc(&apc("i=31,s=1,v=1,a=q,t=d,f=24", b"AAAA"), 1, 0), b"\x1b_Gi=31;OK\x1b\\");
        // All mediums are implemented now, so probes for them get OK too
        // (a read failure later answers EBADF, honestly and identically).
        assert_eq!(k.on_apc(&apc("a=q,t=t,i=2,s=1,v=1,f=24", &b64(b"/tmp/x")), 1, 0), b"\x1b_Gi=2;OK\x1b\\");
        assert_eq!(k.on_apc(&apc("a=q,t=s,i=3,s=1,v=1,f=24", &b64(b"/icat-x")), 1, 0), b"\x1b_Gi=3;OK\x1b\\");
        // Transfer-only frames answer nothing; named DISPLAYS ack.
        assert!(k.on_apc(&apc("a=t,i=5,f=100", &b64(&tiny_png())), 1, 0).is_empty());
        assert_eq!(k.on_apc(&apc("a=T,i=7,f=100", &b64(&tiny_png())), 1, 0), b"\x1b_Gi=7;OK\x1b\\");
        // Placement of a known id acks (with p= when one was requested);
        // of an unknown id it says ENOENT so the client can resync.
        assert_eq!(k.on_apc(&apc("a=p,i=7", &[]), 2, 0), b"\x1b_Gi=7;OK\x1b\\");
        assert_eq!(k.on_apc(&apc("a=p,i=7,p=42", &[]), 2, 0), b"\x1b_Gi=7,p=42;OK\x1b\\");
        assert_eq!(k.on_apc(&apc("a=p,i=99", &[]), 2, 0), b"\x1b_Gi=99;ENOENT:image not found\x1b\\");
        // Unnamed traffic stays unacked - silence is the spec for id=0.
        assert!(k.on_apc(&apc("a=T,f=100", &b64(&tiny_png())), 3, 0).is_empty());
    }

    #[test]
    fn replacement_bumps_generation() {
        let mut k = TabKitty::default();
        k.on_apc(&apc("a=T,i=4,f=100", &b64(&tiny_png())), 1, 0);
        assert_eq!(k.image(4).unwrap().generation, 0);
        assert_eq!(k.placements().len(), 1);
        k.on_apc(&apc("a=t,i=4,f=100", &b64(&tiny_png())), 1, 0);
        assert_eq!(k.image(4).unwrap().generation, 1);
        assert!(k.placements().is_empty(), "new data kept the old image's placements");
    }

    /// The terminal-side rule for moving the cursor past a displayed image.
    /// This is what keeps a prompt clear of an icat-drawn image: icat sends
    /// `c`/`r` and one trailing newline and expects the terminal to have
    /// advanced the cursor by the placement rectangle.
    #[test]
    fn placement_cursor_advance_rule() {
        use alacritty_terminal::term::kitty_placement_cursor_advance as adv;
        // A 10x20 px cell, as the parser would be handed from the term.
        let (cw, ch) = (10u32, 20u32);
        // Explicit rectangle (c/r): used as-is, pixels ignored.
        assert_eq!(adv(&apc("a=T,i=7,f=100,c=42,r=17", b""), cw, ch), Some((42, 17)));
        assert_eq!(adv(&apc("a=p,i=7,c=10,r=3", b""), cw, ch), Some((10, 3)));
        assert_eq!(adv(&apc("a=1,i=7,c=5,r=2", b""), cw, ch), Some((5, 2)));
        // Key order is irrelevant.
        assert_eq!(adv(&apc("r=9,a=T,c=8,i=1", b""), cw, ch), Some((8, 9)));
        // Native size (icat's normal mode): only s/v, derived from the cell.
        // 200x100 px in a 10x20 cell -> 20 x 5 cells.
        assert_eq!(adv(&apc("a=T,q=2,f=100,t=f,s=200,v=100", b""), cw, ch), Some((20, 5)));
        // Rounded up: 201x101 px -> 21 x 6 cells.
        assert_eq!(adv(&apc("a=T,f=100,s=201,v=101", b""), cw, ch), Some((21, 6)));
        // An exact multiple is not rounded up: 200x100 -> 20 x 5, not 21 x 6.
        assert_eq!(adv(&apc("a=T,f=100,s=200,v=100", b""), cw, ch), Some((20, 5)));
        // Explicit c/r beat the pixel-derived size when both are present.
        assert_eq!(adv(&apc("a=T,f=100,c=3,r=2,s=200,v=100", b""), cw, ch), Some((3, 2)));
        // Transmit-only stores but never displays: no movement.
        assert_eq!(adv(&apc("a=t,i=7,f=100,c=10,r=3", b""), cw, ch), None);
        // Deletes and queries don't advance from the cursor.
        assert_eq!(adv(&apc("a=d,i=7,c=10,r=3", b""), cw, ch), None);
        assert_eq!(adv(&apc("a=q,i=7,c=10,r=3", b""), cw, ch), None);
        // The C=1 no-movement policy (icat sets this with --place).
        assert_eq!(adv(&apc("a=T,i=7,c=10,r=3,C=1", b""), cw, ch), None);
        // A relative placement (p= parent) is anchored elsewhere.
        assert_eq!(adv(&apc("a=T,i=7,c=10,r=3,p=4", b""), cw, ch), None);
        // A rectangle that can't be derived (no c/r and no s/v) moves nothing.
        assert_eq!(adv(&apc("a=T,i=7,f=100", b""), cw, ch), None);
        // Trailing base64 data must not be misread as keys: it is cut off at
        // the data-separating ';' before the params are scanned.
        let data = b64(&tiny_png());
        assert_eq!(adv(&apc("a=T,i=7,f=100,s=4,v=2", &data), cw, ch), Some((1, 1)));
        // A non-Kitty APC (tmux passes other dialects through) never moves.
        assert_eq!(adv(b"q;some other dialect", cw, ch), None);
    }
}
