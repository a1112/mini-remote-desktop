# Multi-Protocol Remote Desktop Client (Qt)

A Qt-based remote desktop viewer supporting multiple streaming protocols.

## Features

- **Multiple Protocol Support**
  - WebRTC (via aiortc) - Primary protocol
  - QUIC (via aioquic) - Experimental
  - JPEG Streaming - Fallback for low bandwidth

- **Hardware Acceleration**
  - **Decoding**: DXVA2/D3D11VA/NVDEC/QSV (Windows)
    - NVIDIA NVDEC (h264_nvdec)
    - Intel Quick Sync Video (h264_qsv)
    - D3D11 Video Acceleration (h264_d3d11va)
    - DXVA2 (h264_dxva2)
  - **Rendering**: OpenGL/D3D11 GPU-accelerated display
  - Zero-copy texture update when available
  - Automatic fallback to software decoding

- **Qt-based GUI**
  - PySide6 (Qt 6)
  - Hardware-accelerated video rendering
  - Responsive dark theme

- **Real-time Statistics**
  - Latency (RTT)
  - Bitrate (Mbps)
  - Frame rate (FPS)
  - Packet loss
  - Connection uptime

- **Protocol Negotiation**
  - Automatic protocol selection
  - Fallback on connection failure
  - Capability exchange with agents

## Compatibility

Compatible with mini-remote-desktop agents:
- `agent-rust` (Rust implementation)
- `agent-python` (Python implementation)
- `agent` (Electron implementation)

## Requirements

- Python 3.10+
- Windows 10/11 (for hardware acceleration)
- Or Linux/macOS (with software decoding)

### Hardware Acceleration Requirements

**For NVIDIA NVDEC (h264_nvdec):**
- NVIDIA GPU with Kepler architecture or newer
- NVIDIA GPU driver 418.81 or newer
- CUDA toolkit (for PyAV build)

**For Intel QSV (h264_qsv):**
- Intel CPU with Quick Sync Video support
- Intel Media Driver on Linux
- OneVPL on Windows

**For D3D11VA (h264_d3d11va):**
- Windows 8 or newer
- GPU with D3D11.1 support
- WDDM 1.2 driver or newer

**For DXVA2 (h264_dxva2):**
- Windows 7 or newer
- GPU with DXVA2 support

## Installation

### Standard Installation

```bash
# Navigate to client directory
cd client-qt

# Create virtual environment (recommended)
python -m venv venv
source venv/bin/activate  # On Windows: venv\Scripts\activate

# Install dependencies
pip install -r requirements.txt
```

### Building PyAV with Hardware Acceleration

For maximum hardware acceleration support, build PyAV with custom flags:

```bash
# Install build dependencies
pip install numpy

# Clone PyAV and build with hardware acceleration
git clone https://github.com/PyAV-Org/PyAV.git
cd PyAV

# Build with hardware decoders
python setup.py build --enable-libopenh264

# Install
pip install .
```

Or install pre-built wheel with hardware support (if available):
```bash
pip install av --extra-index-url https://PyAV-Org.github.io/wheels
```

## Configuration

Edit `config.yaml` to customize settings:

```yaml
# Signaling Server
signaling:
  ws_url: "ws://localhost:9527"

# Protocol priority
protocols:
  priority:
    - webrtc
    - quic
    - jpeg
  enable_fallback: true

# Video decoder settings
video:
  decoder:
    hardware_accelerated: true
    low_delay: true
```

## Usage

### Start the Signaling Server

```bash
# In the project root
cd server
node index.js
```

### Start an Agent

```bash
# Rust agent
cd agent-rust
cargo run

# Or Python agent
cd agent-python
python run_nvenc_agent.py
```

### Start the Qt Client

```bash
cd client-qt
python main.py

# With custom signaling server
python main.py --signaling-url ws://192.168.1.100:9527

# With debug logging
python main.py --verbose
```

## Project Structure

```
client-qt/
├── main.py                 # Application entry point
├── config.yaml             # Configuration file
├── requirements.txt        # Python dependencies
├── README.md              # This file
└── src/
    ├── ui/                # Qt UI components
    │   ├── main_window.py     # Main window
    │   ├── video_view.py      # Video display widget (OpenGL/D3D11)
    │   ├── device_panel.py    # Device list panel
    │   └── stats_panel.py     # Statistics panel
    ├── protocols/          # Protocol handlers
    │   ├── base.py            # Base protocol interface
    │   ├── manager.py         # Protocol manager
    │   ├── webrtc/            # WebRTC implementation
    │   ├── quic/              # QUIC implementation
    │   └── jpeg/              # JPEG streaming
    ├── signaling/          # WebSocket signaling
    │   ├── client.py          # Signaling client
    │   └── protocol.py        # Protocol definitions
    ├── decoder/           # Hardware-accelerated decoders
    │   └── hw_decoder.py      # DXVA2/D3D11VA/NVDEC/QSV
    ├── render/            # Hardware-accelerated rendering
    │   └── d3d11_renderer.py  # D3D11 renderer
    └── core/               # Core components
        └── stats.py           # Statistics tracking
```

## Architecture

```
┌─────────────────────────────────────────────────┐
│                  Qt UI Layer                   │
│  ┌─────────────┐  ┌─────────────┐  ┌──────────┐│
│  │ DevicePanel │  │  VideoView  │  │StatsPanel││
│  └─────────────┘  └─────────────┘  └──────────┘│
└─────────────────────────────────────────────────┘
                         ↓
┌─────────────────────────────────────────────────┐
│              Protocol Manager                   │
│  - Protocol negotiation                         │
│  - Fallback handling                            │
│  - Unified interface                            │
└─────────────────────────────────────────────────┘
         ↓               ↓               ↓
┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│   WebRTC     │ │    QUIC      │ │    JPEG      │
│  (aiortc)    │ │  (aioquic)   │ │  (native)    │
└──────────────┘ └──────────────┘ └──────────────┘
                         ↓
┌─────────────────────────────────────────────────┐
│            Signaling Client                     │
│          (WebSocket + JSON)                     │
└─────────────────────────────────────────────────┘
```

## Protocol Details

### WebRTC
- Uses aiortc for peer connection
- H.264 codec via hardware acceleration
- Lowest latency for capable networks

### QUIC
- Uses aioquic for transport
- Experimental support
- Better for unstable networks

### JPEG Streaming
- Fallback via WebSocket signaling
- Simple frame-by-frame delivery
- Higher latency but simpler

## Troubleshooting

### "aiortc not available" error
Make sure aiortc is installed:
```bash
pip install aiortc
```

### Hardware decoder not available

Check available decoders:
```python
from src.decoder.hw_decoder import get_available_decoders
print(get_available_decoders())
```

Expected output (Windows with NVIDIA GPU):
```
['h264_nvdec', 'h264_qsv', 'h264_d3d11va', 'h264_dxva2', 'h264']
```

If only `['h264']` is shown, hardware acceleration is not available.

**Solutions:**
1. Update GPU drivers
2. Rebuild PyAV with hardware support
3. Verify GPU has hardware video acceleration
4. Check `ffmpeg -codecs | grep h264` for available decoders

### Black/blank video display
- Check if the agent is sending H.264 codec
- Verify firewall settings for WebRTC ports
- Try switching to JPEG protocol as fallback
- Check hardware decoder is compatible with incoming stream

### High CPU usage
- Hardware decoder may not be active
- Check if `h264_nvdec` or `h264_qsv` is being used
- Reduce video resolution in agent config
- Lower frame rate in agent config

### Connection timeout
- Verify signaling server is running
- Check network connectivity
- Try disabling firewall temporarily

## License

MIT
