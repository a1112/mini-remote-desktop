"""
Device list panel for selecting and connecting to remote agents.
"""

import logging
from typing import List, Optional, Callable

from PySide6.QtCore import Qt, QObject, Signal, Slot
from PySide6.QtGui import QStandardItemModel, QStandardItem
from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QListView, QPushButton, QFrame, QSizePolicy, QAbstractItemView
)

from ..signaling.protocol import DeviceInfo

logger = logging.getLogger(__name__)


class DevicePanel(QWidget):
    """
    Panel displaying available remote desktop agents.

    Features:
    - Device list with status indicators
    - Connect/Disconnect buttons
    - Auto-refresh on device list changes
    - Filter and search capabilities
    """

    # Signals
    device_selected = Signal(str)  # device_id
    connect_requested = Signal(str)  # device_id
    disconnect_requested = Signal()

    def __init__(self, parent=None):
        """Initialize device panel."""
        super().__init__(parent)

        self._devices: List[DeviceInfo] = []
        self._selected_device_id: Optional[str] = None
        self._is_connected = False

        self._setup_ui()

    def _setup_ui(self) -> None:
        """Setup UI components."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 8, 8, 8)
        layout.setSpacing(8)

        # Header
        header = self._create_header()
        layout.addWidget(header)

        # Device list
        self._device_list = self._create_device_list()
        layout.addWidget(self._device_list)

        # Connection buttons
        self._connect_buttons = self._create_connect_buttons()
        layout.addWidget(self._connect_buttons)

        # Status bar
        self._status_bar = self._create_status_bar()
        layout.addWidget(self._status_bar)

    def _create_header(self) -> QWidget:
        """Create header widget."""
        widget = QWidget()
        layout = QHBoxLayout(widget)
        layout.setContentsMargins(0, 0, 0, 0)

        # Title
        title = QLabel("Devices")
        title.setStyleSheet("""
            QLabel {
                font-size: 14px;
                font-weight: 600;
                color: #e0e0e0;
            }
        """)
        layout.addWidget(title)

        layout.addStretch()

        # Device count
        self._device_count_label = QLabel("0 devices")
        self._device_count_label.setStyleSheet("""
            QLabel {
                font-size: 11px;
                color: #888;
            }
        """)
        layout.addWidget(self._device_count_label)

        return widget

    def _create_device_list(self) -> QListView:
        """Create device list view."""
        list_view = QListView()
        list_view.setMinimumHeight(200)
        list_view.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

        list_view.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        list_view.setStyleSheet("""
            QListView {
                background-color: #2a2a2a;
                border: 1px solid #3a3a3a;
                border-radius: 4px;
                padding: 4px;
            }
            QListView::item {
                padding: 8px;
                border-radius: 4px;
                margin: 2px;
            }
            QListView::item:selected {
                background-color: #3a7bd5;
                color: white;
            }
            QListView::item:hover {
                background-color: #333333;
            }
            QListView::item:selected:hover {
                background-color: #4a8be5;
            }
        """)

        # Setup model
        self._device_model = QStandardItemModel()
        list_view.setModel(self._device_model)

        # Connect selection change
        list_view.selectionModel().selectionChanged.connect(
            self._on_selection_changed
        )

        # Connect double-click to connect
        list_view.doubleClicked.connect(self._on_double_click)

        return list_view

    def _create_connect_buttons(self) -> QWidget:
        """Create connect/disconnect buttons."""
        widget = QWidget()
        layout = QHBoxLayout(widget)
        layout.setContentsMargins(0, 0, 0, 0)

        self._connect_btn = QPushButton("Connect")
        self._connect_btn.setEnabled(False)
        self._connect_btn.setStyleSheet("""
            QPushButton {
                background-color: #3a7bd5;
                color: white;
                border: none;
                border-radius: 4px;
                padding: 8px 16px;
                font-weight: 600;
            }
            QPushButton:hover {
                background-color: #4a8be5;
            }
            QPushButton:disabled {
                background-color: #3a3a3a;
                color: #666;
            }
        """)
        self._connect_btn.clicked.connect(self._on_connect_clicked)
        layout.addWidget(self._connect_btn)

        self._disconnect_btn = QPushButton("Disconnect")
        self._disconnect_btn.setEnabled(False)
        self._disconnect_btn.setStyleSheet("""
            QPushButton {
                background-color: #d64a4a;
                color: white;
                border: none;
                border-radius: 4px;
                padding: 8px 16px;
                font-weight: 600;
            }
            QPushButton:hover {
                background-color: #e65a5a;
            }
            QPushButton:disabled {
                background-color: #3a3a3a;
                color: #666;
            }
        """)
        self._disconnect_btn.clicked.connect(self._on_disconnect_clicked)
        layout.addWidget(self._disconnect_btn)

        return widget

    def _create_status_bar(self) -> QLabel:
        """Create status bar label."""
        label = QLabel("Not connected to signaling server")
        label.setStyleSheet("""
            QLabel {
                font-size: 10px;
                color: #666;
                padding: 4px;
            }
        """)
        return label

    def set_devices(self, devices: List[DeviceInfo]) -> None:
        """
        Update the device list.

        Args:
            devices: List of available devices
        """
        self._devices = devices
        self._device_model.clear()

        for device in devices:
            item = QStandardItem()
            item.setText(device.name)
            item.setEditable(False)
            item.setData(device.id, Qt.ItemDataRole.UserRole)

            # Set icon based on online status
            if device.online:
                status_icon = "●"
                item.setText(f"{status_icon} {device.name}")
            else:
                status_icon = "○"
                item.setText(f"{status_icon} {device.name} (Offline)")

            self._device_model.appendRow(item)

        # Update count
        self._device_count_label.setText(f"{len(devices)} devices")

        # Update status
        if devices:
            self._status_bar.setText(f"{len(devices)} device(s) available")
        else:
            self._status_bar.setText("No devices available")

    def set_connected(self, device_id: str, connected: bool) -> None:
        """
        Update connection state.

        Args:
            device_id: Connected device ID
            connected: True if connected, False otherwise
        """
        self._is_connected = connected
        self._selected_device_id = device_id if connected else None

        self._connect_btn.setEnabled(not connected and self._selected_device_id is not None)
        self._disconnect_btn.setEnabled(connected)

        if connected:
            device = next((d for d in self._devices if d.id == device_id), None)
            name = device.name if device else device_id
            self._status_bar.setText(f"Connected to {name}")
        else:
            self._status_bar.setText("Disconnected")

    def set_signaling_connected(self, connected: bool) -> None:
        """
        Update signaling server connection status.

        Args:
            connected: True if connected to signaling server
        """
        if connected:
            self._status_bar.setText("Connected to signaling server")
        else:
            self._status_bar.setText("Not connected to signaling server")
            self._device_model.clear()
            self._device_count_label.setText("0 devices")

    def _on_selection_changed(self) -> None:
        """Handle device list selection change."""
        indexes = self._device_list.selectionModel().selectedIndexes()
        if indexes:
            index = indexes[0]
            device_id = index.data(Qt.ItemDataRole.UserRole)
            self._selected_device_id = device_id
            self.device_selected.emit(device_id)

            # Enable connect button if not connected
            if not self._is_connected:
                self._connect_btn.setEnabled(True)
        else:
            self._selected_device_id = None
            self._connect_btn.setEnabled(False)

    def _on_double_click(self, index) -> None:
        """Handle double-click on device."""
        device_id = index.data(Qt.ItemDataRole.UserRole)
        if device_id and not self._is_connected:
            self.connect_requested.emit(device_id)

    def _on_connect_clicked(self) -> None:
        """Handle connect button click."""
        if self._selected_device_id and not self._is_connected:
            self.connect_requested.emit(self._selected_device_id)

    def _on_disconnect_clicked(self) -> None:
        """Handle disconnect button click."""
        self.disconnect_requested.emit()

    @property
    def selected_device_id(self) -> Optional[str]:
        """Get currently selected device ID."""
        return self._selected_device_id

    @property
    def is_connected(self) -> bool:
        """Check if connected to a device."""
        return self._is_connected
