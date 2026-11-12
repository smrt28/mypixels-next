"""Native close requests for tests on an isolated X11 display."""
import ctypes as C
import os


def request_close(wid):
    # Send WM_DELETE_WINDOW directly: Xvfb has no window manager to relay
    # xdotool's _NET_CLOSE_WINDOW request.
    class ClientMessage(C.Structure):
        _fields_ = [('type', C.c_int), ('serial', C.c_ulong), ('send_event', C.c_int),
                    ('display', C.c_void_p), ('window', C.c_ulong),
                    ('message_type', C.c_ulong), ('format', C.c_int), ('data', C.c_long * 5)]

    class XEvent(C.Union):
        _fields_ = [('client', ClientMessage), ('pad', C.c_long * 24)]

    x11 = C.CDLL('libX11.so.6')
    x11.XOpenDisplay.argtypes, x11.XOpenDisplay.restype = [C.c_char_p], C.c_void_p
    x11.XInternAtom.argtypes, x11.XInternAtom.restype = [C.c_void_p, C.c_char_p, C.c_int], C.c_ulong
    x11.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(XEvent)]
    x11.XFlush.argtypes = x11.XCloseDisplay.argtypes = [C.c_void_p]
    display = x11.XOpenDisplay(os.environ['DISPLAY'].encode())
    assert display
    try:
        event = XEvent()
        event.client.type, event.client.window, event.client.format = 33, int(wid), 32
        event.client.display = display
        event.client.message_type = x11.XInternAtom(display, b'WM_PROTOCOLS', 0)
        event.client.data[0] = x11.XInternAtom(display, b'WM_DELETE_WINDOW', 0)
        assert x11.XSendEvent(display, int(wid), 0, 0, C.byref(event))
        x11.XFlush(display)
    finally:
        x11.XCloseDisplay(display)
