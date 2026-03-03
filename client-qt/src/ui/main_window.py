"""
Main application window for the remote desktop client.
"""

import asyncio
import logging
from pathlib import Path
from typing import Optional

from PySide6.QtCore import Qt, QTimer, QSettings
from PySide6.QtGui import QCloseEvent
from PySide6.QtWidgets import (
    QMainWindow, QWidget, QHBoxLayout, QVBoxLayout,
    QSplitter, QStatusBar, QMenuBar, QMenu, QApplication,
    QLabel, QComboBox
)

import numpy as np
import numpy.typing as npt

from ..core.stats import Stats, ConnectionState
from ..signaling.client import SignalingClient, SignalingConfig
from ..protocols.manager import ProtocolManager, ProtocolConfig
from .video_view_factory import create_video_view
from .device_panel import DevicePanel
from .stats_panel import StatsPanel

logger = logging.getLogger(__name__)


class MainWindow(QMainWindow):
    """
    Main application window.

    Coordinates between:
    - Signaling client (WebSocket)
    - Protocol manager (WebRTC/QUIC/JPEG)
    - UI components (Video, Device list, Stats)
    """

    def __init__(self):
        """Initialize main window."""
        super().__init__()
        self._initializing_controls = True

        # Configuration
        self._load_config()

        # Core components
        self._signaling = SignalingClient(SignalingConfig(
            ws_url=self._config.get("signaling", {}).get("ws_url", "ws://localhost:9527")
        ))
        self._protocol_manager = ProtocolManager(ProtocolConfig(
            priority=self._config.get("protocols", {}).get("priority", ["webrtc", "quic", "jpeg"]),
            enable_fallback=self._config.get("protocols", {}).get("enable_fallback", True),
            webrtc_use_hw_decoder=self._config.get("video", {}).get("decoder", {}).get("hardware_accelerated", True),
            webrtc_decoder_priority=self._config.get("video", {}).get("decoder", {}).get("decoder_priority", []),
            webrtc_decoder_low_delay=self._config.get("video", {}).get("decoder", {}).get("low_delay", True),
        ))
        self._stats = Stats()

        # Event loop reference
        self._event_loop = asyncio.get_event_loop()
        self._pending_tasks = []

        # Setup UI
        self._setup_ui()
        self._setup_menu()
        self._initializing_controls = False

        # Connect signals
        self._connect_signals()

        # Auto-connect to signaling
        QTimer.singleShot(500, self._connect_to_signaling)

    def _load_config(self) -> dict:
        """Load configuration from file."""
        import yaml

        config_path = Path(__file__).parent.parent.parent / "config.yaml"

        default_config = {
            "signaling": {"ws_url": "ws://localhost:9527"},
            "protocols": {"priority": ["webrtc", "quic", "jpeg"]},
            "ui": {"theme": "dark"}
        }

        if config_path.exists():
            try:
                with open(config_path, 'r') as f:
                    self._config = yaml.safe_load(f)
            except Exception as e:
                logger.warning(f"Failed to load config: {e}")
                self._config = default_config
        else:
            self._config = default_config

        return self._config

    def _setup_ui(self) -> None:
        """Setup UI components."""
        self.setWindowTitle("Remote Desktop Viewer")
        self.setMinimumSize(1000, 600)

        # Apply theme
        self._apply_theme()

        # Central widget
        central = QWidget()
        self.setCentralWidget(central)

        # Main layout
        main_layout = QHBoxLayout(central)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)

        # Create splitter for resizable panels
        splitter = QSplitter(Qt.Orientation.Horizontal)
        main_layout.addWidget(splitter)

        # Left panel (device list + stats)
        left_panel = self._create_left_panel()
        splitter.addWidget(left_panel)

        # Right panel (controls + video view)
        renderer_pref = self._config.get("video", {}).get("display", {}).get("renderer", "auto")
        self._video_view, self._video_backend = create_video_view(renderer_pref)
        right_panel = QWidget()
        right_layout = QVBoxLayout(right_panel)
        right_layout.setContentsMargins(0, 0, 0, 0)
        right_layout.setSpacing(0)
        right_layout.addWidget(self._create_stream_control_bar(), 0)
        right_layout.addWidget(self._video_view, 1)
        splitter.addWidget(right_panel)

        # Set splitter sizes
        splitter.setStretchFactor(0, 0)
        splitter.setStretchFactor(1, 1)
        splitter.setSizes([280, 720])

        # Status bar
        self._status_bar = self.statusBar()
        self._status_bar.showMessage(
            f"Ready | renderer={self._video_backend} | hw-decoder={'on' if self._protocol_manager.config.webrtc_use_hw_decoder else 'off'}"
        )

    def _create_stream_control_bar(self) -> QWidget:
        """
        Create runtime control bar for agent capture/encode switching.
        """
        bar = QWidget()
        bar.setStyleSheet("background-color: #242424; border-bottom: 1px solid #3a3a3a;")
        layout = QHBoxLayout(bar)
        layout.setContentsMargins(8, 6, 8, 6)
        layout.setSpacing(6)

        def add_label(text: str) -> None:
            label = QLabel(text)
            label.setStyleSheet("color: #bfbfbf; font-size: 11px;")
            layout.addWidget(label)

        def add_combo(items: list[str], on_change) -> QComboBox:
            combo = QComboBox()
            combo.addItems(items)
            combo.setMinimumWidth(96)
            combo.setStyleSheet(
                "QComboBox { background-color: #2f2f2f; color: #e0e0e0; border: 1px solid #4a4a4a; padding: 3px 6px; }"
            )
            combo.currentIndexChanged.connect(on_change)
            return combo

        add_label("RES")
        self._ctrl_resolution = add_combo(
            ["native", "1920x1080", "2560x1440", "3840x2160", "1280x720"],
            self._on_resolution_changed,
        )
        layout.addWidget(self._ctrl_resolution)

        add_label("WIN")
        self._ctrl_window = add_combo(["auto", "foreground"], self._on_window_mode_changed)
        layout.addWidget(self._ctrl_window)

        add_label("BR")
        self._ctrl_bitrate = add_combo(["8000", "12000", "20000", "30000", "50000"], self._on_bitrate_changed)
        self._ctrl_bitrate.setCurrentText("20000")
        layout.addWidget(self._ctrl_bitrate)

        add_label("CAP")
        self._ctrl_capture = add_combo(["dxgi", "wgc", "auto"], self._on_capture_backend_changed)
        layout.addWidget(self._ctrl_capture)

        add_label("ENC")
        self._ctrl_encoder = add_combo(["nvenc", "openh264", "auto"], self._on_encoder_changed)
        layout.addWidget(self._ctrl_encoder)

        layout.addStretch(1)
        return bar

    def _current_target_device(self) -> Optional[str]:
        return self._protocol_manager.connected_device_id or self._device_panel.selected_device_id

    def _send_capture_patch(self, patch: dict) -> None:
        target = self._current_target_device()
        if not target:
            self._status_bar.showMessage("No target device selected for control update")
            return
        self._run_async(self._send_capture_patch_async(target, patch))

    async def _send_capture_patch_async(self, target_device_id: str, patch: dict) -> None:
        try:
            await self._signaling.send_capture_update(target_device_id, patch)
            self._status_bar.showMessage(f"Control sent: {patch}")
        except Exception as e:
            logger.error("Failed to send capture patch: %s", e)
            self._status_bar.showMessage(f"Control send failed: {e}")

    def _on_resolution_changed(self, _index: int) -> None:
        if self._initializing_controls:
            return
        val = self._ctrl_resolution.currentText()
        if val == "native":
            self._send_capture_patch({"targetWidth": 0, "targetHeight": 0})
            return
        if "x" in val:
            w, h = val.split("x", 1)
            try:
                self._send_capture_patch({"targetWidth": int(w), "targetHeight": int(h)})
            except ValueError:
                pass

    def _on_window_mode_changed(self, _index: int) -> None:
        if self._initializing_controls:
            return
        self._send_capture_patch({"windowMode": self._ctrl_window.currentText()})

    def _on_bitrate_changed(self, _index: int) -> None:
        if self._initializing_controls:
            return
        try:
            br = int(self._ctrl_bitrate.currentText())
        except ValueError:
            return
        self._send_capture_patch({"bitrateKbps": br})

    def _on_capture_backend_changed(self, _index: int) -> None:
        if self._initializing_controls:
            return
        self._send_capture_patch({"backend": self._ctrl_capture.currentText()})

    def _on_encoder_changed(self, _index: int) -> None:
        if self._initializing_controls:
            return
        self._send_capture_patch({"encoder": self._ctrl_encoder.currentText()})

    def _create_left_panel(self) -> QWidget:
        """Create left panel with device list and stats."""
        panel = QWidget()
        panel.setMinimumWidth(250)
        panel.setMaximumWidth(400)

        layout = QVBoxLayout(panel)
        layout.setContentsMargins(8, 8, 8, 8)
        layout.setSpacing(8)

        # Device panel
        self._device_panel = DevicePanel()
        layout.addWidget(self._device_panel, 1)

        # Stats panel
        self._stats_panel = StatsPanel()
        layout.addWidget(self._stats_panel, 0)

        return panel

    def _setup_menu(self) -> None:
        """Setup menu bar."""
        menubar = self.menuBar()

        # File menu
        file_menu = menubar.addMenu("&File")

        settings_action = file_menu.addAction("&Settings")
        settings_action.triggered.connect(self._show_settings)

        file_menu.addSeparator()

        exit_action = file_menu.addAction("E&xit")
        exit_action.triggered.connect(self.close)

        # Connection menu
        conn_menu = menubar.addMenu("&Connection")

        self._connect_action = conn_menu.addAction("&Connect")
        self._connect_action.setEnabled(False)
        self._connect_action.triggered.connect(self._on_connect_action)

        self._disconnect_action = conn_menu.addAction("&Disconnect")
        self._disconnect_action.setEnabled(False)
        self._disconnect_action.triggered.connect(self._on_disconnect_action)

        # View menu
        view_menu = menubar.addMenu("&View")

        fit_action = view_menu.addAction("&Fit to Window")
        fit_action.setCheckable(True)
        fit_action.setChecked(True)
        fit_action.triggered.connect(lambda: self._video_view.set_scale_mode("fit"))

        fill_action = view_menu.addAction("&Fill Window")
        fill_action.setCheckable(True)
        fill_action.triggered.connect(lambda: self._video_view.set_scale_mode("fill"))

        stretch_action = view_menu.addAction("&Stretch")
        stretch_action.setCheckable(True)
        stretch_action.triggered.connect(lambda: self._video_view.set_scale_mode("stretch"))

        # Help menu
        help_menu = menubar.addMenu("&Help")

        about_action = help_menu.addAction("&About")
        about_action.triggered.connect(self._show_about)

    def _apply_theme(self) -> None:
        """Apply theme to the application."""
        theme = self._config.get("ui", {}).get("theme", "dark")

        if theme == "dark":
            stylesheet = """
                QMainWindow {
                    background-color: #1a1a1a;
                }
                QWidget {
                    background-color: #1a1a1a;
                    color: #e0e0e0;
                }
                QMenuBar {
                    background-color: #2a2a2a;
                    border-bottom: 1px solid #3a3a3a;
                }
                QMenuBar::item {
                    background-color: transparent;
                    padding: 6px 12px;
                }
                QMenuBar::item:selected {
                    background-color: #3a3a3a;
                }
                QMenu {
                    background-color: #2a2a2a;
                    border: 1px solid #3a3a3a;
                }
                QMenu::item {
                    padding: 6px 24px;
                }
                QMenu::item:selected {
                    background-color: #3a7bd5;
                }
                QStatusBar {
                    background-color: #2a2a2a;
                    border-top: 1px solid #3a3a3a;
                    color: #888;
                }
                QSplitter::handle {
                    background-color: #2a2a2a;
                    width: 2px;
                }
                QSplitter::handle:hover {
                    background-color: #3a3a3a;
                }
            """
            self.setStyleSheet(stylesheet)

    def _connect_signals(self) -> None:
        """Connect signals between components."""
        # Device panel signals
        self._device_panel.device_selected.connect(self._on_device_selected)
        self._device_panel.connect_requested.connect(self._on_device_connect)
        self._device_panel.disconnect_requested.connect(self._on_device_disconnect)

        # Protocol manager signals
        self._protocol_manager.on_frame_received(self._on_video_frame)
        self._protocol_manager.on_stats_update(self._on_stats_update)
        self._protocol_manager.on_state_change(self._on_connection_state)

        # Signaling client signals
        self._signaling.on("connected", self._on_signaling_connected)
        self._signaling.on("registered", self._on_signaling_registered)
        self._signaling.on("device_list", self._on_device_list)
        self._signaling.on("device_offline", self._on_device_offline)
        self._signaling.on("answer", self._on_webrtc_answer)
        self._signaling.on("ice_candidate", self._on_ice_candidate)
        self._signaling.on("disconnected", self._on_signaling_disconnected)
        self._signaling.on("error", self._on_signaling_error)

        # Stats signals
        self._stats.register_callback(self._on_stats_callback)

    def _connect_to_signaling(self) -> None:
        """Connect to signaling server."""
        self._status_bar.showMessage("Connecting to signaling server...")

        async def do_connect():
            success = await self._signaling.connect()
            if success:
                client_name = self._config.get("client", {}).get("name", "Qt Controller")
                await self._signaling.register(client_name)

        self._run_async(do_connect())

    def _on_signaling_connected(self, device_id: str) -> None:
        """Handle signaling connection."""
        logger.info(f"Connected to signaling server: {device_id}")
        self._status_bar.showMessage("Connected to signaling server")
        self._device_panel.set_signaling_connected(True)

    def _on_signaling_registered(
        self,
        device_id: str,
        device_list: list
    ) -> None:
        """Handle registration."""
        logger.info(f"Registered: {device_id}")
        self._device_panel.set_devices(device_list)

    def _on_device_list(self, device_list: list) -> None:
        """Handle device list update."""
        from ...signaling.protocol import DeviceInfo
        devices = [DeviceInfo.from_dict(d) if isinstance(d, dict) else d for d in device_list]
        self._device_panel.set_devices(devices)

    def _on_device_offline(self, device_id: str) -> None:
        """Handle device offline."""
        if self._protocol_manager.connected_device_id == device_id:
            self._run_async(self._protocol_manager.disconnect())
            self._video_view.clear_frame()

    def _on_device_selected(self, device_id: str) -> None:
        """Handle device selection."""
        self._connect_action.setEnabled(True)
        self._status_bar.showMessage(f"Selected: {device_id}")

    def _on_device_connect(self, device_id: str) -> None:
        """Handle connect request."""
        self._status_bar.showMessage(f"Connecting to {device_id}...")
        self._run_async(self._connect_to_device(device_id))

    async def _connect_to_device(self, device_id: str) -> None:
        """Connect to remote device."""
        try:
            # Create offer (we use a simple SDP for now)
            # In production, this would be a proper WebRTC offer
            offer_sdp = self._create_dummy_offer()

            # Connect via protocol manager
            success = await self._protocol_manager.connect(
                device_id,
                offer_sdp,
                self._signaling
            )

            if success:
                self._device_panel.set_connected(device_id, True)
                self._stats.set_device_info(device_id, device_id)
                self._stats.set_protocol(self._protocol_manager.current_protocol)
                self._connect_action.setEnabled(False)
                self._disconnect_action.setEnabled(True)

        except Exception as e:
            logger.error(f"Connection failed: {e}")
            self._status_bar.showMessage(f"Connection failed: {e}")

    def _create_dummy_offer(self) -> str:
        """Create a dummy SDP offer for testing."""
        # This is a minimal SDP offer
        # In production, this would be created properly
        return """v=0
o=- 0 0 IN IP4 127.0.0.1
s=-
t=0 0
a=fingerprint:sha-256 AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA
a=group:BUNDLE 0
a=extmap-allow-mixed
a=msid-semantic: WMS
m=video 9 UDP/TLS/RTP/SAVPF 96
c=IN IP4 0.0.0.0
a=sendrecv
a=mid:0
a=rtcp-mux
a=rtpmap:96 H264/90000
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f
a=setup:actpass
"""

    def _on_webrtc_answer(self, answer_sdp: str) -> None:
        """Handle WebRTC answer from agent."""
        logger.info("Received SDP answer")
        # Answer is handled by protocol manager

    def _on_ice_candidate(self, candidate: dict) -> None:
        """Handle ICE candidate."""
        self._run_async(self._protocol_manager.add_ice_candidate(candidate))

    def _on_video_frame(self, frame: npt.NDArray[np.uint8]) -> None:
        """Handle received video frame."""
        self._video_view.set_frame(frame)
        self._stats.add_received_frame()

    def _on_stats_update(self, stats: dict) -> None:
        """Handle stats update from protocol handler."""
        self._stats.update_network_stats(
            latency_ms=stats.get("latency_ms", 0),
            packet_loss=stats.get("packet_loss", 0),
            jitter_ms=stats.get("jitter_ms", 0)
        )

    def _on_connection_state(self, state: ConnectionState) -> None:
        """Handle connection state change."""
        self._stats.update_connection_state(state)

    def _on_stats_callback(self, stats) -> None:
        """Handle stats update."""
        self._stats_panel.set_stats(stats)

    def _on_device_disconnect(self) -> None:
        """Handle disconnect request."""
        self._run_async(self._protocol_manager.disconnect())
        self._video_view.clear_frame()
        self._device_panel.set_connected("", False)
        self._connect_action.setEnabled(True)
        self._disconnect_action.setEnabled(False)
        self._stats_panel.clear_stats()

    def _on_connect_action(self) -> None:
        """Handle connect menu action."""
        if self._device_panel.selected_device_id:
            self._on_device_connect(self._device_panel.selected_device_id)

    def _on_disconnect_action(self) -> None:
        """Handle disconnect menu action."""
        self._on_device_disconnect()

    def _on_signaling_disconnected(self) -> None:
        """Handle signaling disconnection."""
        logger.warning("Disconnected from signaling server")
        self._device_panel.set_signaling_connected(False)
        self._status_bar.showMessage("Disconnected from signaling server")

    def _on_signaling_error(self, error: str) -> None:
        """Handle signaling error."""
        logger.error(f"Signaling error: {error}")
        self._status_bar.showMessage(f"Error: {error}")

    def _run_async(self, coro) -> None:
        """Run async coroutine in a background thread."""
        import concurrent.futures

        def run_in_thread():
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            try:
                loop.run_until_complete(coro)
            finally:
                loop.close()

        # Run in thread pool to avoid blocking Qt
        import threading
        thread = threading.Thread(target=run_in_thread, daemon=True)
        thread.start()

    def _show_settings(self) -> None:
        """Show settings dialog."""
        self._status_bar.showMessage("Settings dialog not yet implemented")

    def _show_about(self) -> None:
        """Show about dialog."""
        from PySide6.QtWidgets import QMessageBox
        QMessageBox.about(
            self,
            "About Remote Desktop Viewer",
            "<h3>Remote Desktop Viewer</h3>"
            "<p>Multi-protocol remote desktop client</p>"
            "<p>Protocols: WebRTC, QUIC, JPEG</p>"
            "<p>Version: 0.1.0</p>"
        )

    def closeEvent(self, event: QCloseEvent) -> None:
        """Handle window close event."""
        # Disconnect everything
        async def cleanup():
            await self._protocol_manager.disconnect()
            await self._signaling.disconnect()

        self._run_async(cleanup())

        # Cancel pending tasks
        for task in self._pending_tasks:
            if not task.done():
                task.cancel()

        event.accept()
