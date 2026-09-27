package com.a1112.rdeskmobile;

import static org.junit.Assert.*;
import org.junit.Test;

public class ProtocolTest {
    @Test public void acceptsOnlyPrivateOrLoopbackGateways() {
        assertEquals("ws://192.168.1.5:9533/mobile/desktop/ws", Protocol.desktopUrl("192.168.1.5", 9533));
        assertEquals("ws://127.0.0.1:9533/mobile/phone/publish/ws", Protocol.phonePublishUrl("127.0.0.1", 9533));
        assertThrows(IllegalArgumentException.class, () -> Protocol.desktopUrl("example.com", 9533));
        assertThrows(IllegalArgumentException.class, () -> Protocol.desktopUrl("192.168.1.5/x", 9533));
    }

    @Test public void normalizesTouchesAndClampsEdges() {
        assertEquals(0.5f, Protocol.normalized(50f, 100f), 0.0001f);
        assertEquals(0f, Protocol.normalized(-5f, 100f), 0f);
        assertEquals(1f, Protocol.normalized(105f, 100f), 0f);
    }

    @Test public void validatesJpegBeforePublishing() {
        assertTrue(Protocol.isJpeg(new byte[] {(byte)0xff, (byte)0xd8, 1, (byte)0xff, (byte)0xd9}));
        assertFalse(Protocol.isJpeg(new byte[] {1, 2, 3}));
        assertFalse(Protocol.isJpeg(new byte[Protocol.MAX_JPEG_BYTES + 1]));
    }

    @Test public void probesNearbyHomeSubnetsWhenWifiUsesAnotherSubnet() {
        assertEquals(java.util.List.of("192.168.10.", "192.168.0.", "192.168.1."),
                Protocol.discoveryPrefixes("192.168.10.102"));
        assertEquals(java.util.List.of("10.5.7."), Protocol.discoveryPrefixes("10.5.7.23"));
        assertTrue(Protocol.discoveryPrefixes("8.8.8.8").isEmpty());
    }
}
