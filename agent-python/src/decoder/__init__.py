"""
Video decoder module for testing and verification.

Provides H.264 decoding using PyAV for validating encoded frames.
"""

from .pyav_decoder import (
    PyAVDecoder,
    DecodedFrame,
    create_decoder,
)

__all__ = [
    'PyAVDecoder',
    'DecodedFrame',
    'create_decoder',
]
