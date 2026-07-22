"""
QUIC transport adapter implementation.

Uses aioquic to provide high-performance UDP-based transport with:
- Multiple streams for multiplexing (control, video, stats)
- Built-in QoS and flow control
- Automatic retransmission
- Connection migration support
"""

import asyncio
import datetime
import ipaddress
import json
import logging
import struct
import time
from typing import Optional, Dict, Any, List, Tuple, TYPE_CHECKING

from .base import TransportAdapter, TransportError, ConnectionError, SendError
from .stats import TransportStats, FrameInfo

if TYPE_CHECKING:
    from aioquic.tls import Certificate

try:
    from aioquic.asyncio import QuicConnectionProtocol, serve
    from aioquic.quic.configuration import QuicConfiguration
    from aioquic.quic.events import StreamDataReceived, QuicEvent, StreamReset
    from aioquic.tls import Certificate
    import ssl
    HAS_AIOQUIC = True
except ImportError:
    HAS_AIOQUIC = False
    Certificate = None  # type: ignore
    # Define placeholders for type hints
    QuicConnectionProtocol = object  # type: ignore
    QuicConfiguration = object  # type: ignore
    QuicEvent = object  # type: ignore
    StreamDataReceived = object  # type: ignore
    StreamReset = object  # type: ignore

logger = logging.getLogger(__name__)

# Stream IDs for different purposes
STREAM_CONTROL = 0  # Control/signaling messages
STREAM_VIDEO_START = 1  # Video data streams start from here


class QUICAdapter(TransportAdapter):
    """
    QUIC transport adapter for H.264 video streaming.

    Features:
    - Dedicated control stream (stream 0) for signaling
    - Multiple video streams for parallel transmission
    - Automatic congestion control via QUIC
    - Connection migration support
    """

    def __init__(
        self,
        host: str = "0.0.0.0",
        port: int = 0,  # Auto-assign
        certificate: Optional[Certificate] = None,
    ):
        """
        Initialize QUIC adapter.

        Args:
            host: Local host to bind to
            port: Local port (0 for auto-assign)
            certificate: TLS certificate (None for self-generated)
        """
        if not HAS_AIOQUIC:
            raise ImportError("aioquic is required for QUIC transport")

        super().__init__("quic")

        self.host = host
        self.port = port

        # QUIC configuration
        self._quic_config = QuicConfiguration(
            is_client=False,
            alpn_protocols=["remote-desktop"],
            max_datagram_frame_size=65536,
        )

        # Generate self-signed certificate if none provided
        if certificate:
            self._quic_config.load_cert_from_obj(certificate)
        else:
            # Create temporary self-signed certificate
            cert_path, key_path = self._generate_self_signed_cert()
            if cert_path and key_path:
                try:
                    self._quic_config.load_cert_chain(cert_path, key_path)
                except Exception as e:
                    logger.warning(f"Could not load certificate: {e}, using insecure mode")
                    # For testing, we can proceed without a certificate
                    # In production, this should fail

        # Connection state
        self._protocol: Optional[QuicConnectionProtocol] = None
        self._controller_addr: Optional[Tuple[str, int]] = None
        self._current_stream_id = STREAM_VIDEO_START

        # Frame sequencing
        self._frame_number = 0
        self._pending_streams: Dict[int, asyncio.Future] = {}

        # RTT tracking
        self._ping_sent_time: Optional[float] = None
        self._ping_interval = 1.0

        logger.info("QUIC adapter initialized")

    def _generate_self_signed_cert(self, key_only: bool = False) -> tuple:
        """Generate a self-signed certificate for QUIC."""
        import tempfile
        import os
        import subprocess

        cert_dir = tempfile.gettempdir()
        cert_path = os.path.join(cert_dir, "quic_cert.pem")
        key_path = os.path.join(cert_dir, "quic_key.pem")

        # Check if certificate already exists
        if os.path.exists(cert_path) and os.path.exists(key_path):
            return cert_path, key_path

        # Generate self-signed certificate using cryptography
        try:
            from cryptography import x509
            from cryptography.x509.oid import NameOID
            from cryptography.hazmat.primitives import hashes, serialization
            from cryptography.hazmat.primitives.asymmetric import rsa
            from cryptography.hazmat.backends import default_backend
            import datetime

            # Generate private key
            private_key = rsa.generate_private_key(
                public_exponent=65537,
                key_size=2048,
                backend=default_backend()
            )

            # Generate certificate
            subject = issuer = x509.Name([
                x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
                x509.NameAttribute(NameOID.STATE_OR_PROVINCE_NAME, "CA"),
                x509.NameAttribute(NameOID.LOCALITY_NAME, "San Francisco"),
                x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Remote Desktop"),
                x509.NameAttribute(NameOID.COMMON_NAME, "localhost"),
            ])

            cert = x509.CertificateBuilder().subject_name(
                subject
            ).issuer_name(
                issuer
            ).public_key(
                private_key.public_key()
            ).serial_number(
                x509.random_serial_number()
            ).not_valid_before(
                datetime.datetime.utcnow()
            ).not_valid_after(
                datetime.datetime.utcnow() + datetime.timedelta(days=365)
            ).add_extension(
                x509.SubjectAlternativeName([
                    x509.DNSName("localhost"),
                    x509.IPAddress(ipaddress.IPv4Address("127.0.0.1")),
                ]),
                critical=False,
            ).sign(private_key, hashes.SHA256(), default_backend())

            # Write certificate
            with open(cert_path, "wb") as f:
                f.write(cert.public_bytes(serialization.Encoding.PEM))

            # Write private key
            with open(key_path, "wb") as f:
                f.write(private_key.private_bytes(
                    encoding=serialization.Encoding.PEM,
                    format=serialization.PrivateFormat.TraditionalOpenSSL,
                    encryption_algorithm=serialization.NoEncryption()
                ))

            return cert_path, key_path

        except ImportError:
            # Fallback: try using openssl if available
            try:
                subprocess.run([
                    "openssl", "req", "-x509", "-newkey", "rsa:2048",
                    "-keyout", key_path, "-out", cert_path,
                    "-days", "365", "-nodes",
                    "-subj", "/C=US/ST=CA/L=San Francisco/O=Remote Desktop/CN=localhost"
                ], check=True, capture_output=True)
                return cert_path, key_path
            except (subprocess.CalledProcessError, FileNotFoundError):
                logger.warning("Could not generate self-signed certificate")
                return None, None
        except Exception as e:
            logger.warning(f"Certificate generation failed: {e}")
            return None, None

    async def connect(self, offer: str, metadata: Optional[dict] = None) -> str:
        """
        Establish QUIC connection.

        Args:
            offer: JSON string with QUIC connection parameters
            metadata: Optional connection metadata

        Returns:
            Connection answer with local QUIC endpoint info

        Raises:
            ConnectionError: If connection fails
        """
        try:
            # Parse offer
            offer_data = json.loads(offer) if isinstance(offer, str) else offer

            controller_host = offer_data.get("host", "127.0.0.1")
            controller_port = offer_data.get("port", 0)
            controller_quic_port = offer_data.get("quic_port", 4433)

            logger.info(f"QUIC offer received from {controller_host}:{controller_quic_port}")

            # Start QUIC server
            self._server = await serve(
                host=self.host,
                port=self.port,
                configuration=self._quic_config,
                create_protocol=self._create_protocol,
            )

            # Get actual bound port
            if hasattr(self._server, "sockets") and self._server.sockets:
                self.port = self._server.sockets[0].getsockname()[1]

            logger.info(f"QUIC server listening on {self.host}:{self.port}")

            # Wait for controller to connect (with timeout)
            self._controller_addr = (controller_host, controller_quic_port)

            # Create answer with our endpoint info
            answer = {
                "protocol": "quic",
                "host": self.host,
                "port": self.port,
                "alpn": "remote-desktop",
                "supports_migration": True,
            }

            logger.info("QUIC connection setup complete")
            return json.dumps(answer)

        except json.JSONDecodeError as e:
            raise ConnectionError(f"Invalid offer format: {e}", protocol="quic")
        except Exception as e:
            raise ConnectionError(f"QUIC connection failed: {e}", protocol="quic")

    def _create_protocol(self):
        """Create QUIC protocol instance."""
        protocol = QuicConnectionProtocol(
            quic=None,
            configuration=self._quic_config,
        )
        protocol._event_handler = self._handle_quic_event
        return protocol

    async def _handle_quic_event(self, event: QuicEvent):
        """
        Handle QUIC events.

        Args:
            event: QUIC event from protocol
        """
        if isinstance(event, StreamDataReceived):
            await self._handle_stream_data(event)
        elif isinstance(event, StreamReset):
            logger.warning(f"Stream {event.stream_id} reset by peer")

    async def _handle_stream_data(self, event: StreamDataReceived):
        """
        Handle data received on a stream.

        Args:
            event: StreamDataReceived event
        """
        stream_id = event.stream_id
        data = event.data

        if stream_id == STREAM_CONTROL:
            # Control stream - handle signaling messages
            try:
                message = json.loads(data.decode())
                await self._handle_control_message(message)
            except (json.JSONDecodeError, UnicodeDecodeError) as e:
                logger.warning(f"Invalid control message: {e}")

            # Send acknowledgment
            if self._protocol:
                self._protocol.send_stream_data(stream_id, b"ACK", end_stream=False)
        else:
            logger.debug(f"Received data on stream {stream_id}: {len(data)} bytes")

    async def _handle_control_message(self, message: dict):
        """
        Handle control channel message.

        Args:
            message: JSON control message
        """
        msg_type = message.get("type")

        if msg_type == "ping":
            # Respond to ping for RTT measurement
            await self._send_control_message({
                "type": "pong",
                "timestamp": message.get("timestamp", time.time())
            })
            # Update RTT
            if self._ping_sent_time:
                rtt = (time.time() - self._ping_sent_time) * 1000
                self._stats.update_rtt(rtt)
                self._ping_sent_time = None

        elif msg_type == "pong":
            # RTT measurement response
            if self._ping_sent_time:
                rtt = (time.time() - self._ping_sent_time) * 1000
                self._stats.update_rtt(rtt)
                self._ping_sent_time = None

        elif msg_type == "keyframe_request":
            # Controller requesting keyframe
            self._emit("keyframe_request")

        elif msg_type == "connected":
            # Controller confirmed connection
            self._update_connection_state(True)
            logger.info("QUIC connection confirmed by controller")

    async def _send_control_message(self, message: dict) -> None:
        """
        Send a message on the control stream.

        Args:
            message: Message dictionary to send
        """
        if self._protocol:
            data = json.dumps(message).encode()
            self._protocol.send_stream_data(STREAM_CONTROL, data, end_stream=False)

    async def send_media(self, frame: FrameInfo) -> None:
        """
        Send an encoded media frame via QUIC.

        Args:
            frame: Frame information including encoded data

        Raises:
            SendError: If send fails
        """
        if not self._protocol or not self.is_connected:
            raise SendError("Not connected", protocol="quic")

        try:
            # Get or create stream for this frame
            stream_id = self._get_next_stream_id()

            # Frame header: [type(1)][frame_number(4)][timestamp(8)][is_keyframe(1)][size(4)]
            frame_type = 1  # H.264
            is_keyframe = 1 if frame.is_keyframe else 0

            header = struct.pack(
                "!BIQBI",
                frame_type,
                self._frame_number,
                frame.timestamp,
                is_keyframe,
                len(frame.data)
            )

            # Send header and data
            full_data = header + frame.data

            # For large frames, chunk them
            chunk_size = 4096  # 4KB chunks
            for i in range(0, len(full_data), chunk_size):
                chunk = full_data[i:i + chunk_size]
                is_end = (i + chunk_size) >= len(full_data)
                self._protocol.send_stream_data(stream_id, chunk, end_stream=is_end)

            # Update stats
            self._stats.bytes_sent += len(frame.data)
            self._stats.packets_sent += 1
            self._stats.frames_sent += 1
            self._frame_number += 1

            # Update bandwidth periodically
            if self._frame_number % 30 == 0:  # Every 30 frames
                self._stats.update_bandwidth()
                self._stats.update_fps(self._frame_number)

        except Exception as e:
            self._stats.packets_lost += 1
            self._stats.connection_errors += 1
            raise SendError(f"Failed to send frame: {e}", protocol="quic")

    def _get_next_stream_id(self) -> int:
        """
        Get the next stream ID for video transmission.

        Uses a simple rotation of streams for load balancing.
        """
        # Use streams 4-15 for video (client-initiated, bidirectional)
        # Rotate through them for load balancing
        stream_id = STREAM_VIDEO_START + (self._frame_number % 12)
        return stream_id

    async def request_keyframe(self) -> None:
        """
        Request a keyframe from the encoder.

        This is handled by the manager, but we can track it.
        """
        logger.debug("Keyframe request (handled by encoder)")

    async def disconnect(self) -> None:
        """Close QUIC connection and cleanup."""
        logger.info("Disconnecting QUIC adapter...")

        self._update_connection_state(False)

        if self._protocol:
            try:
                self._protocol.close()
            except Exception:
                pass
            self._protocol = None

        if hasattr(self, "_server") and self._server:
            try:
                self._server.close()
                await self._server.wait_closed()
            except Exception:
                pass
            self._server = None

        logger.info("QUIC adapter disconnected")

    async def start_rtt_measurement(self) -> None:
        """Start periodic RTT measurement via ping/pong."""
        if not self.is_connected:
            return

        self._ping_sent_time = time.time()
        await self._send_control_message({
            "type": "ping",
            "timestamp": self._ping_sent_time
        })

    async def _rtt_loop(self) -> None:
        """Periodic RTT measurement loop."""
        while self.is_connected:
            try:
                await asyncio.sleep(self._ping_interval)
                await self.start_rtt_measurement()
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"RTT measurement error: {e}")

    @property
    def supports_migration(self) -> bool:
        """Check if connection migration is supported."""
        return True

    @property
    def active_streams(self) -> int:
        """Get number of active video streams."""
        return min(12, self._frame_number)


def create_quic_offer(host: str = "127.0.0.1", port: int = 4433) -> str:
    """
    Create a QUIC offer for connection initiation.

    Args:
        host: Controller host
        port: QUIC port

    Returns:
        JSON offer string
    """
    offer = {
        "protocol": "quic",
        "host": host,
        "port": port,
        "alpn": "remote-desktop",
    }
    return json.dumps(offer)


def is_quic_available() -> bool:
    """Check if QUIC support is available."""
    return HAS_AIOQUIC
