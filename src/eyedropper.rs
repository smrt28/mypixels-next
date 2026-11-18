//! Screen-wide color selection belongs to the desktop compositor, including
//! pixels from other applications on Wayland. Keep portal I/O off the UI thread.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{atomic::{AtomicU64, Ordering}, mpsc};
use std::time::{Duration, Instant};

use futures::{channel::oneshot, FutureExt, StreamExt};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::config::Rgb;

pub type PickResult = Result<Option<Rgb>, String>;

pub struct Picker {
    pub result: mpsc::Receiver<PickResult>,
    // Dropping the UI request also closes the desktop's selection overlay.
    _cancel: oneshot::Sender<()>,
}

impl Picker {
    #[cfg(test)]
    pub fn completed(value: PickResult) -> Self {
        let (tx, result) = mpsc::channel();
        tx.send(value).unwrap();
        let (cancel, _) = oneshot::channel();
        Self { result, _cancel: cancel }
    }

    pub fn start(wake: impl FnOnce() + Send + 'static) -> Result<Self, String> {
        let (tx, result) = mpsc::channel();
        let (cancel, cancelled) = oneshot::channel();
        std::thread::Builder::new().name("eyedropper".into()).spawn(move || {
            let picked = futures::executor::block_on(pick(cancelled));
            if tx.send(picked).is_ok() { wake(); }
        }).map_err(|e| format!("Cannot start screen color picker: {e}"))?;
        Ok(Self { result, _cancel: cancel })
    }
}

async fn pick(mut cancelled: oneshot::Receiver<()>) -> PickResult {
    // Prefer a transparent selector wherever the compositor supports both
    // helpers. Desktop names do not determine which protocols are available.
    if let (Some(slurp), Some(grim)) = (find_tool("slurp"), find_tool("grim")) {
        if transparent_selection_supported() {
            return pick_transparent(&slurp, &grim, &mut cancelled);
        }
    }
    let result = async {
        let conn = zbus::connection::Builder::session()?
            .method_timeout(Duration::from_secs(5)).build().await?;
        pick_on(&conn, &mut cancelled).await
    }.await;
    result.map_err(|e: zbus::Error| format!("Screen color picker unavailable: {e}"))
}

fn transparent_selection_supported() -> bool {
    use wayland_client::{Connection, Dispatch, QueueHandle, globals::{registry_queue_init, GlobalListContents},
        protocol::wl_registry};
    struct Probe;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
        fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event,
            _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() { return false; }
    let Ok(conn) = Connection::connect_to_env() else { return false; };
    let Ok((globals, _queue)) = registry_queue_init::<Probe>(&conn) else { return false; };
    globals.contents().with_list(|list| selection_protocols_supported(
        list.iter().map(|global| (global.interface.as_str(), global.version))))
}

fn selection_protocols_supported<'a>(globals: impl IntoIterator<Item = (&'a str, u32)>) -> bool {
    let (mut selection, mut capture) = (false, false);
    for (interface, version) in globals {
        if version == 0 { continue; }
        match interface {
            "zwlr_layer_shell_v1" => selection = true,
            "zwlr_screencopy_manager_v1" => capture = true,
            _ => (),
        }
    }
    selection && capture
}

fn find_tool(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name))
        .find(|path| path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

// Own each helper until it exits, including when Settings closes mid-selection.
struct Helper(Child);
impl Drop for Helper {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn run_helper(command: &mut Command, cancelled: &mut oneshot::Receiver<()>,
    timeout: Option<Duration>) -> Result<Option<(bool, Vec<u8>)>, String> {
    if !matches!(cancelled.try_recv(), Ok(None)) { return Ok(None); }
    let mut child = Helper(command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("Cannot start screen color picker: {e}"))?);
    let started = Instant::now();
    loop {
        if !matches!(cancelled.try_recv(), Ok(None)) { return Ok(None); }
        if let Some(status) = child.0.try_wait().map_err(|e| format!("Screen color picker failed: {e}"))? {
            let mut bytes = Vec::new();
            // Helpers only return a point or a tiny PNG, never a full screen.
            child.0.stdout.take().unwrap().take(65537).read_to_end(&mut bytes)
                .map_err(|e| format!("Cannot read screen color: {e}"))?;
            if bytes.len() > 65536 { return Err("Screen color response is too large".into()); }
            return Ok(Some((status.success(), bytes)));
        }
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            return Err("Screen color capture timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pick_transparent(slurp: &Path, grim: &Path, cancelled: &mut oneshot::Receiver<()>) -> PickResult {
    // All painted parts are transparent, so even an animated layer's removal
    // cannot change the sampled pixel. Only the pointer changes to a crosshair.
    let selection = run_helper(Command::new(slurp).args([
        "-p", "-b", "#00000000", "-c", "#00000000", "-s", "#00000000", "-B", "#00000000",
        "-w", "0", "-f", "%x,%y",
    ]), cancelled, None)?;
    let Some((success, point)) = selection else { return Ok(None); };
    if !success { return Ok(None); } // Escape/right-click cancels slurp.
    let point = std::str::from_utf8(&point).map_err(|_| "Invalid screen pixel location")?;
    let (x, y) = point.trim().split_once(',').ok_or("Invalid screen pixel location")?;
    let (x, y) = (x.parse::<i32>().map_err(|_| "Invalid screen pixel location")?,
        y.parse::<i32>().map_err(|_| "Invalid screen pixel location")?);
    let geometry = format!("{x},{y} 1x1");
    // Wait for slurp to exit before capturing, omit the cursor, and retain the
    // selected output's native scale (no downsampling of HiDPI text).
    let capture = run_helper(Command::new(grim).args(["-g", &geometry, "-t", "png", "-"]),
        cancelled, Some(Duration::from_secs(5)))?;
    let Some((success, bytes)) = capture else { return Ok(None); };
    if !success { return Err("Cannot capture the selected screen pixel".into()); }
    pixel_from_png(&bytes).map(Some)
}

fn pixel_from_png(bytes: &[u8]) -> Result<Rgb, String> {
    let invalid = |_| "Invalid screen pixel image".to_string();
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(invalid)?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > 32 || h > 32 { return Err("Invalid screen pixel image size".into()); }
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).map_err(invalid)?;
    // A logical 1x1 region may contain multiple native pixels on HiDPI screens.
    let offset = ((h / 2 * w + w / 2) * info.color_type.samples() as u32) as usize;
    match info.color_type {
        png::ColorType::Rgb | png::ColorType::Rgba => Ok(Rgb(pixels[offset], pixels[offset + 1], pixels[offset + 2])),
        png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha => Ok(Rgb(pixels[offset], pixels[offset], pixels[offset])),
        _ => Err("Invalid screen pixel image format".into()),
    }
}

async fn pick_on(conn: &zbus::Connection, cancelled: &mut oneshot::Receiver<()>) -> zbus::Result<Option<Rgb>> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let token = format!("mypixels_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let sender = conn.unique_name().expect("session bus assigns a unique name").as_str()
        .trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let request = zbus::Proxy::new(conn, "org.freedesktop.portal.Desktop", path.as_str(),
        "org.freedesktop.portal.Request").await?;
    // Subscribe before PickColor: a cancellation/error response may arrive
    // before the method reply. handle_token makes the path predictable.
    let mut responses = request.receive_signal("Response").await?;
    let portal = zbus::Proxy::new(conn, "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Screenshot").await?;
    if !matches!(cancelled.try_recv(), Ok(None)) { return Ok(None); }
    let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
    let handle: OwnedObjectPath = portal.call("PickColor", &("", options)).await?;
    if handle.as_str() != path {
        return Err(zbus::Error::Failure("Desktop returned an unexpected color picker handle".into()));
    }
    let response = futures::select! {
        response = responses.next().fuse() => response,
        _ = cancelled.fuse() => {
            let _: zbus::Result<()> = request.call("Close", &()).await;
            return Ok(None);
        }
    }.ok_or_else(|| zbus::Error::Failure("Desktop color picker disconnected".into()))?;
    let (status, values): (u32, HashMap<String, OwnedValue>) = response.body().deserialize()?;
    decode_response(status, values).map_err(zbus::Error::Failure)
}

fn decode_response(status: u32, mut values: HashMap<String, OwnedValue>) -> PickResult {
    match status {
        1 => return Ok(None), // User cancelled; leave the original color alone.
        0 => (),
        _ => return Err("The desktop could not pick a screen color".into()),
    }
    let value = values.remove("color").ok_or("Desktop returned no color")?;
    let (r, g, b): (f64, f64, f64) = value.try_into().map_err(|_| "Desktop returned an invalid color")?;
    if [r, g, b].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err("Desktop returned an invalid color".into());
    }
    let channel = |v: f64| (v * 255.0).round() as u8;
    Ok(Some(Rgb(channel(r), channel(g), channel(b))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};

    #[test]
    fn direct_selection_requires_both_compositor_protocols() {
        let layer_shell = ("zwlr_layer_shell_v1", 1);
        let screencopy = ("zwlr_screencopy_manager_v1", 3);
        assert!(selection_protocols_supported([layer_shell, screencopy]));
        // Selection alone (or capture alone) must leave the portal available.
        assert!(!selection_protocols_supported([layer_shell]));
        assert!(!selection_protocols_supported([screencopy]));
        assert!(!selection_protocols_supported([("wl_compositor", 6), ("wl_shm", 1)]));
        assert!(!selection_protocols_supported([layer_shell, (screencopy.0, 0)]));
    }

    fn png_image(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(pixels).unwrap();
        }
        bytes
    }

    #[test]
    fn screen_pixels_preserve_exact_bytes_and_native_hidpi_samples() {
        // These channel bytes are whitespace in a binary PPM. Decode PNG as an
        // image, never run text/whitespace normalization over pixel data.
        assert_eq!(pixel_from_png(&png_image(1, 1, &[9, 10, 32])).unwrap(), Rgb(9, 10, 32));
        let pixels = [255, 255, 255, 0, 0, 0, 255, 255, 255, 12, 34, 56];
        assert_eq!(pixel_from_png(&png_image(2, 2, &pixels)).unwrap(), Rgb(12, 34, 56));
        assert!(pixel_from_png(b"invalid PNG").is_err());
    }

    #[test]
    fn transparent_helpers_disable_all_tints_and_capture_only_after_selection() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mypixels-transparent-picker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let slurp = dir.join("slurp");
        let grim = dir.join("grim");
        let script = |path: &Path, text| {
            std::fs::write(path, text).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        std::fs::write(dir.join("pixel.png"), png_image(1, 1, &[9, 10, 32])).unwrap();
        script(&slurp, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{0}/selection-args'\nprintf '%s\\n' '-12,34'\ntouch '{0}/selected'\n", dir.display()));
        script(&grim, format!("#!/bin/sh\ntest -f '{0}/selected' || exit 8\nprintf '%s\\n' \"$@\" > '{0}/capture-args'\nexec cat '{0}/pixel.png'\n", dir.display()));
        let (_cancel, mut cancelled) = oneshot::channel();
        assert_eq!(pick_transparent(&slurp, &grim, &mut cancelled).unwrap(), Some(Rgb(9, 10, 32)));
        let args = std::fs::read_to_string(dir.join("selection-args")).unwrap();
        assert_eq!(args, "-p\n-b\n#00000000\n-c\n#00000000\n-s\n#00000000\n-B\n#00000000\n-w\n0\n-f\n%x,%y\n");
        assert_eq!(std::fs::read_to_string(dir.join("capture-args")).unwrap(), "-g\n-12,34 1x1\n-t\npng\n-\n");
        std::fs::remove_file(dir.join("capture-args")).unwrap();
        script(&slurp, "#!/bin/sh\nexit 1\n".to_owned());
        assert_eq!(pick_transparent(&slurp, &grim, &mut cancelled).unwrap(), None);
        assert!(!dir.join("capture-args").exists(), "cancelling must not capture the screen");
        let (cancel, mut cancelled) = oneshot::channel();
        drop(cancel);
        script(&slurp, "#!/bin/sh\nexit 99\n".to_owned());
        assert_eq!(pick_transparent(&slurp, &grim, &mut cancelled).unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn color_values(color: (f64, f64, f64)) -> HashMap<String, OwnedValue> {
        HashMap::from([("color".into(), Value::from(color).try_to_owned().unwrap())])
    }

    #[test]
    fn portal_colors_round_to_srgb_bytes_and_invalid_results_are_rejected() {
        assert_eq!(decode_response(0, color_values((0.0, 0.5, 1.0))).unwrap(), Some(Rgb(0, 128, 255)));
        assert_eq!(decode_response(1, HashMap::new()).unwrap(), None);
        assert!(decode_response(2, HashMap::new()).is_err());
        assert!(decode_response(0, HashMap::new()).is_err());
        for value in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            assert!(decode_response(0, color_values((value, 0.0, 0.0))).is_err());
        }
        assert!(decode_response(0, HashMap::from([("color".into(), OwnedValue::from(42u32))])).is_err());
    }

    struct Bus(Child);
    impl Drop for Bus {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }

    struct Portal {
        status: u32,
        // Holding the response lets the test cancel an in-flight request.
        started: Mutex<Option<oneshot::Sender<()>>>,
        closed: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    }
    struct Request(Arc<Mutex<Option<oneshot::Sender<()>>>>);

    #[zbus::interface(name = "org.freedesktop.portal.Request")]
    impl Request {
        fn close(&self) {
            if let Some(tx) = self.0.lock().unwrap().take() { let _ = tx.send(()); }
        }
    }

    #[zbus::interface(name = "org.freedesktop.portal.Screenshot")]
    impl Portal {
        async fn pick_color(&self, parent: &str, options: HashMap<String, OwnedValue>,
            #[zbus(connection)] conn: &zbus::Connection,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            assert!(parent.is_empty());
            let token = <&str>::try_from(&options["handle_token"]).unwrap();
            let sender = header.sender().unwrap().as_str().trim_start_matches(':').replace('.', "_");
            let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
            conn.object_server().at(path.as_str(), Request(self.closed.clone())).await.unwrap();
            if let Some(tx) = self.started.lock().unwrap().take() {
                let _ = tx.send(());
            } else {
                // Intentionally signal BEFORE returning the method result.
                conn.emit_signal(None::<&str>, path.as_str(), "org.freedesktop.portal.Request", "Response",
                    &(self.status, color_values((0.2, 0.4, 0.6)))).await.unwrap();
            }
            Ok(path.try_into().unwrap())
        }
    }

    #[test]
    fn private_portal_handles_early_responses_and_closes_cancelled_requests() {
        let mut bus = Bus(Command::new("dbus-daemon").args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped()).spawn().expect("test requires dbus-daemon"));
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap()).read_line(&mut address).unwrap();
        futures::executor::block_on(async {
            for status in [0, 1, 2, 3] {
                let (started_tx, started_rx) = oneshot::channel();
                let (closed_tx, closed_rx) = oneshot::channel();
                let server = zbus::connection::Builder::address(address.trim()).unwrap()
                    .name("org.freedesktop.portal.Desktop").unwrap()
                    .serve_at("/org/freedesktop/portal/desktop", Portal {
                        status, started: Mutex::new((status == 3).then_some(started_tx)),
                        closed: Arc::new(Mutex::new(Some(closed_tx))),
                    }).unwrap().build().await.unwrap();
                let client = zbus::connection::Builder::address(address.trim()).unwrap().build().await.unwrap();
                let (cancel_tx, mut cancel_rx) = oneshot::channel();
                if status == 3 {
                    let cancel = async {
                        started_rx.await.unwrap();
                        drop(cancel_tx);
                        closed_rx.await.unwrap();
                    };
                    let (result, ()) = futures::join!(pick_on(&client, &mut cancel_rx), cancel);
                    assert_eq!(result.unwrap(), None);
                } else {
                    let result = pick_on(&client, &mut cancel_rx).await;
                    match status {
                        0 => assert_eq!(result.unwrap(), Some(Rgb(51, 102, 153))),
                        1 => assert_eq!(result.unwrap(), None),
                        2 => assert!(result.is_err()),
                        _ => unreachable!(),
                    }
                    drop(cancel_tx);
                }
                server.close().await.unwrap();
            }
        });
    }
}
