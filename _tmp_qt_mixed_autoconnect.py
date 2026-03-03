import os
import sys
import time

from PySide6.QtCore import QTimer
from PySide6.QtWidgets import QApplication

BASE = r"J:\ProjectTest\远程探查\mini-remote-desktop\client-qt"
sys.path.insert(0, BASE)

from src.ui.main_window import MainWindow

app = QApplication(sys.argv)
win = MainWindow()
win.show()

attempts = {"n": 0}


def try_connect():
    attempts["n"] += 1
    try:
        devices = [d for d in win._device_panel._devices if getattr(d, "online", True)]
        if devices and not win._protocol_manager.is_connected:
            win._on_device_connect(devices[0].id)
    except Exception as e:
        print(f"qt_auto_connect_error={e}", flush=True)
    if attempts["n"] >= 20:
        connect_timer.stop()


def finish():
    conn = win._stats.connection
    print(
        "qt_result state={state} protocol={protocol} frames={frames} fps={fps:.2f} bytes={bytes}".format(
            state=conn.state.value,
            protocol=conn.protocol,
            frames=conn.total_frames_received,
            fps=conn.fps,
            bytes=conn.total_bytes_received,
        ),
        flush=True,
    )
    try:
        win.close()
    except Exception:
        pass
    app.quit()

connect_timer = QTimer()
connect_timer.timeout.connect(try_connect)
connect_timer.start(1000)

QTimer.singleShot(40000, finish)

sys.exit(app.exec())
