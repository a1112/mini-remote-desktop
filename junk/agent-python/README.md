# agent-python

High-performance Python agent for mini-remote-desktop.

Compatible with signaling-rs and controller-rust.

## Features

- Windows screen capture via d3dshot (DirectX)
- H.264 encoding with PyAV (software/hardware acceleration)
- WebRTC support via aiortc
- Compatible with existing signaling protocol

## Requirements

- Python 3.10+
- Windows (for d3dshot screen capture)

## Installation

```bash
# Install dependencies
pip install -r requirements.txt

# Or using pip with extras
pip install -e ".[windows]"
```

## Usage

```bash
# Run the agent
python src/main.py
```

## Configuration

Edit `config.json` to customize:

```json
{
  "ws_url": "ws://127.0.0.1:9527",
  "device_name": "Python Agent",
  "capture": {
    "fps": 30,
    "target_width": 1920,
    "target_height": 1080,
    "bitrate_kbps": 5000
  }
}
```

## Architecture

```
+------------------+     +------------------+     +------------------+
|  Controller      | <-->|  Signaling       | <-->|   Agent (Python) |
|  (controller-    |     |  Server          |     |   (this project) |
|   rust)          |     |  (signaling-rs)  |     |                   |
+------------------+     +------------------+     +------------------+
        ^                                                    |
        |              WebRTC (H.264 over RTP)               |
        +----------------------------------------------------+
```

## Project Structure

```
agent-python/
├── pyproject.toml          # Project configuration
├── requirements.txt        # Python dependencies
├── config.json            # Agent configuration
├── src/
│   ├── main.py            # Entry point
│   ├── config.py          # Configuration management
│   ├── signaling/         # WebSocket signaling client
│   ├── capture/           # Screen capture (d3dshot)
│   ├── encoder/           # H.264 encoder (PyAV)
│   ├── webrtc/            # WebRTC peer management
│   └── utils/             # Utilities (stats, etc.)
```

## License

MIT
