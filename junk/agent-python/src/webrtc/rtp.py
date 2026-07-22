"""
RTP packetization for H.264 video streams.

Implements RFC 6184: RTP Payload Format for H.264 Video.
Supports:
- Single NALU packet
- Fragmentation Unit (FU-A) for large NALUs
- Stap-A (Single-time Aggregation Packet)
"""

import struct
import logging
from dataclasses import dataclass
from typing import List, Optional

logger = logging.getLogger(__name__)


# NALU types (RFC 6184 Section 5.3)
NALU_TYPE_UNSPECIFIED = 0
NALU_TYPE_SLICE = 1
NALU_TYPE_DPA = 2
NALU_TYPE_DPB = 3
NALU_TYPE_DPC = 4
NALU_TYPE_IDR = 5
NALU_TYPE_SEI = 6
NALU_TYPE_SPS = 7
NALU_TYPE_PPS = 8
NALU_TYPE_AUD = 9
NALU_TYPE_END_OF_SEQUENCE = 10
NALU_TYPE_END_OF_STREAM = 11
NALU_TYPE_FILLER = 12
NALU_TYPE_SPSEXT = 13
NALU_TYPE_PREFIX = 14
NALU_TYPE_SUB_SPS = 15
NALU_TYPE_DPS = 16
NALU_TYPE_RESERVED = 17
NALU_TYPE_FU_A = 28
NALU_TYPE_FU_B = 29
NALU_TYPE_STAP_A = 24
NALU_TYPE_STAP_B = 25
NALU_TYPE_MTAP16 = 26
NALU_TYPE_MTAP24 = 27


@dataclass
class RTPPacket:
    """An RTP packet."""
    payload: bytes
    sequence_number: int
    timestamp: int
    marker: bool = False  # Last packet of a frame
    payload_type: int = 96  # Dynamic payload type for H.264


@dataclass
class NALU:
    """A Network Abstraction Layer Unit."""
    data: bytes
    nalu_type: int
    ref_idc: int

    @staticmethod
    def parse(data: bytes) -> 'NALU':
        """Parse NALU from raw bytes."""
        if len(data) == 0:
            raise ValueError("Empty NALU")

        # NALU header (1 byte)
        # +---------------+
        # |0|1|2|3|4|5|6|7|
        # +-+-+-+-+-+-+-+-+
        # |F|NRI|  Type   |
        # +---------------+
        header = data[0]
        nalu_ref_idc = (header >> 5) & 0x03
        nalu_type = header & 0x1F

        return NALU(
            data=data,
            nalu_type=nalu_type,
            ref_idc=nalu_ref_idc
        )

    @property
    def is_keyframe(self) -> bool:
        """Check if this is a keyframe NALU."""
        return self.nalu_type == NALU_TYPE_IDR

    @property
    def is_parameter_set(self) -> bool:
        """Check if this is SPS or PPS."""
        return self.nalu_type in (NALU_TYPE_SPS, NALU_TYPE_PPS)


class H264RTPPacketizer:
    """
    Packetizes H.264 NALUs into RTP packets.

    Follows RFC 6184 with FU-A fragmentation for large NALUs.
    """

    def __init__(
        self,
        mtu: int = 1200,
        payload_type: int = 96,
        clock_rate: int = 90000
    ):
        """
        Initialize the packetizer.

        Args:
            mtu: Maximum Transmission Unit (payload size per packet)
            payload_type: RTP payload type (96 is typical for H.264)
            clock_rate: RTP clock rate in Hz (90kHz for video)
        """
        self.mtu = mtu
        self.payload_type = payload_type
        self.clock_rate = clock_rate
        self._sequence_number = 0
        self._timestamp = 0
        self._last_timestamp_ms = 0

    def packetize(
        self,
        encoded_frame: bytes,
        timestamp_ms: int,
        is_keyframe: bool = False
    ) -> List[RTPPacket]:
        """
        Packetize an encoded H.264 frame into RTP packets.

        Args:
            encoded_frame: Raw H.264 NALU(s) (with start codes)
            timestamp_ms: Frame timestamp in milliseconds
            is_keyframe: Whether this is a keyframe

        Returns:
            List of RTP packets ready to send
        """
        # Update timestamp (90kHz clock)
        if self._last_timestamp_ms == 0:
            self._timestamp = 0
        else:
            delta = timestamp_ms - self._last_timestamp_ms
            self._timestamp += int(delta * self.clock_rate / 1000)

        self._last_timestamp_ms = timestamp_ms

        # Parse NALUs from frame
        nalus = self._split_nalus(encoded_frame)

        if not nalus:
            logger.warning("No NALUs found in encoded frame")
            return []

        packets = []

        for i, nalu in enumerate(nalus):
            # Is this the last NALU?
            is_last = (i == len(nalus) - 1)

            # Packetize this NALU
            nalu_packets = self._packetize_nalu(nalu, is_last)
            packets.extend(nalu_packets)

        return packets

    def _split_nalus(self, data: bytes) -> List[NALU]:
        """
        Split H.264 bitstream into individual NALUs.

        Handles Annex B start codes: 0x000001, 0x00000001
        """
        nalus = []
        i = 0

        while i < len(data):
            # Find start code
            if i + 3 <= len(data) and data[i:i+3] == b'\x00\x00\x01':
                start = i + 3
            elif i + 4 <= len(data) and data[i:i+4] == b'\x00\x00\x00\x01':
                start = i + 4
            else:
                i += 1
                continue

            # Find next start code or end
            end = start
            while end < len(data):
                if end + 3 <= len(data) and data[end:end+3] == b'\x00\x00\x01':
                    break
                if end + 4 <= len(data) and data[end:end+4] == b'\x00\x00\x00\x01':
                    break
                end += 1

            # Extract NALU
            nalu_data = data[start:end]
            if nalu_data:
                nalus.append(NALU.parse(nalu_data))

            i = end

        return nalus

    def _packetize_nalu(self, nalu: NALU, is_last_nalu: bool) -> List[RTPPacket]:
        """
        Packetize a single NALU into one or more RTP packets.

        Uses FU-A fragmentation if NALU exceeds MTU.
        """
        nalu_data = nalu.data
        nalu_size = len(nalu_data)

        # Header + payload fits in one packet
        if nalu_size <= self.mtu:
            packet = RTPPacket(
                payload=nalu_data,
                sequence_number=self._sequence_number,
                timestamp=self._timestamp,
                marker=is_last_nalu,
                payload_type=self.payload_type
            )
            self._sequence_number = (self._sequence_number + 1) & 0xFFFF
            return [packet]

        # Need FU-A fragmentation
        return self._fragment_fu_a(nalu, is_last_nalu)

    def _fragment_fu_a(self, nalu: NALU, is_last_nalu: bool) -> List[RTPPacket]:
        """
        Fragment NALU using FU-A (Fragmentation Unit A).

        FU-A header structure:
        +---------------+
        |0|1|2|3|4|5|6|7|
        +-+-+-+-+-+-+-+-+
        |F|NRI|  Type   |
        +---------------+
        F=1, Type=28 (FU-A)

        FU indicator (1 byte) + FU header (1 byte) + payload
        """
        nalu_data = nalu.data
        nalu_header = nalu_data[0]

        # Maximum payload per FU-A packet (minus FU header)
        max_payload = self.mtu - 2

        packets = []
        offset = 1  # Skip original NALU header
        total_size = len(nalu_data)
        fragment_num = 0

        while offset < total_size:
            chunk_size = min(max_payload, total_size - offset)
            is_start = (fragment_num == 0)
            is_end = (offset + chunk_size >= total_size)

            # FU indicator
            fu_indicator = (nalu_header & 0xE0) | NALU_TYPE_FU_A

            # FU header
            # +---------------+
            # |0|1|2|3|4|5|6|7|
            # +-+-+-+-+-+-+-+-+
            # |S|E|R|  Type   |
            # +---------------+
            fu_header = 0
            if is_start:
                fu_header |= 0x80  # S bit
            if is_end:
                fu_header |= 0x40  # E bit
            fu_header |= (nalu_header & 0x1F)  # Original NALU type

            # Build payload
            payload = bytes([fu_indicator, fu_header]) + nalu_data[offset:offset + chunk_size]

            packet = RTPPacket(
                payload=payload,
                sequence_number=self._sequence_number,
                timestamp=self._timestamp,
                marker=is_end and is_last_nalu,
                payload_type=self.payload_type
            )
            packets.append(packet)

            self._sequence_number = (self._sequence_number + 1) & 0xFFFF
            offset += chunk_size
            fragment_num += 1

        return packets

    def reset(self) -> None:
        """Reset sequence number and timestamp."""
        self._sequence_number = 0
        self._timestamp = 0
        self._last_timestamp_ms = 0


class H264RTPDepacketizer:
    """
    Depacketizes H.264 RTP packets back into NALUs.

    Handles single NALU, FU-A reassembly, and STAP-A.
    """

    def __init__(self):
        """Initialize the depacketizer."""
        self._fu_buffer = {}  # (timestamp, ssrc) -> (fu_header, data_parts)
        self._expected_seq = {}  # Track expected sequence numbers

    def depacketize(self, packet: RTPPacket) -> Optional[bytes]:
        """
        Depacketize an RTP packet.

        Args:
            packet: RTP packet to depacketize

        Returns:
            Reconstructed NALU data (with start code) if complete, None otherwise
        """
        payload = packet.payload

        if len(payload) < 1:
            return None

        # Parse payload header (first byte)
        nalu_header = payload[0]
        nalu_type = nalu_header & 0x1F

        # Single NALU packet
        if nalu_type < 24:
            return self._add_start_code(payload)

        # STAP-A (aggregation)
        elif nalu_type == NALU_TYPE_STAP_A:
            return self._depacketize_stap_a(payload)

        # FU-A (fragmentation)
        elif nalu_type == NALU_TYPE_FU_A:
            return self._depacketize_fu_a(packet)

        # Other types not supported
        else:
            logger.warning(f"Unsupported NALU type: {nalu_type}")
            return None

    def _add_start_code(self, nalu_data: bytes) -> bytes:
        """Add Annex B start code to NALU."""
        return b'\x00\x00\x00\x01' + nalu_data

    def _depacketize_stap_a(self, payload: bytes) -> Optional[bytes]:
        """
        Depacketize STAP-A (Single-Time Aggregation Packet).

        STAP-A contains multiple NALUs in one RTP packet.
        """
        if len(payload) < 2:
            return None

        result = b''
        offset = 1  # Skip STAP-A header

        while offset < len(payload):
            # Read NALU size (2 bytes, big endian)
            if offset + 2 > len(payload):
                break

            nalu_size = struct.unpack('>H', payload[offset:offset + 2])[0]
            offset += 2

            if offset + nalu_size > len(payload):
                logger.warning("Invalid STAP-A packet")
                break

            nalu_data = payload[offset:offset + nalu_size]
            result += self._add_start_code(nalu_data)
            offset += nalu_size

        return result if result else None

    def _depacketize_fu_a(self, packet: RTPPacket) -> Optional[bytes]:
        """
        Depacketize FU-A (Fragmentation Unit A).

        Reassembles fragmented NALUs from multiple RTP packets.
        """
        payload = packet.payload

        if len(payload) < 2:
            return None

        fu_indicator = payload[0]
        fu_header = payload[1]

        start = (fu_header & 0x80) != 0
        end = (fu_header & 0x40) != 0
        original_nalu_type = fu_header & 0x1F

        key = (packet.timestamp, 0)  # ssrc not used in this context

        # Start fragment
        if start:
            fu_data = payload[2:]
            original_header = (fu_indicator & 0xE0) | original_nalu_type
            self._fu_buffer[key] = (original_header, [fu_data])
            return None

        # Continuation or end fragment
        if key in self._fu_buffer:
            original_header, parts = self._fu_buffer[key]
            parts.append(payload[2:])

            # End fragment - reassemble
            if end:
                del self._fu_buffer[key]
                nalu_data = bytes([original_header]) + b''.join(parts)
                return self._add_start_code(nalu_data)

            return None

        # Fragments without start
        logger.warning("Received FU fragment without start")
        return None

    def reset(self) -> None:
        """Clear all buffers."""
        self._fu_buffer.clear()
        self._expected_seq.clear()


def create_h264_packetizer(mtu: int = 1200) -> H264RTPPacketizer:
    """
    Create an H.264 RTP packetizer with default settings.

    Args:
        mtu: Maximum payload size (default 1200 bytes)

    Returns:
        Configured H264RTPPacketizer
    """
    return H264RTPPacketizer(
        mtu=mtu,
        payload_type=96,
        clock_rate=90000
    )
