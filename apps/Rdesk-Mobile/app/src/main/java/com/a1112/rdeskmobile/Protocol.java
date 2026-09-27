package com.a1112.rdeskmobile;

import java.util.ArrayList;
import java.util.List;

final class Protocol {
    static final int MAX_JPEG_BYTES = 2 * 1024 * 1024;

    private Protocol() {}

    static String desktopUrl(String host, int port) {
        return base(host, port) + "/mobile/desktop/ws";
    }

    static String phonePublishUrl(String host, int port) {
        return base(host, port) + "/mobile/phone/publish/ws";
    }

    private static String base(String host, int port) {
        if (host == null || port < 1 || port > 65535) throw new IllegalArgumentException("无效的地址或端口");
        String[] parts = host.trim().split("\\.", -1);
        if (parts.length != 4) throw new IllegalArgumentException("请输入局域网 IPv4 地址");
        int[] octets = new int[4];
        for (int i = 0; i < 4; i++) {
            if (!parts[i].matches("[0-9]{1,3}")) throw new IllegalArgumentException("无效的 IPv4 地址");
            octets[i] = Integer.parseInt(parts[i]);
            if (octets[i] > 255) throw new IllegalArgumentException("无效的 IPv4 地址");
        }
        boolean local = octets[0] == 10 || octets[0] == 127 ||
                (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31) ||
                (octets[0] == 192 && octets[1] == 168);
        if (!local) throw new IllegalArgumentException("仅允许受信局域网地址");
        return "ws://" + host.trim() + ":" + port;
    }

    static float normalized(float position, float extent) {
        if (!Float.isFinite(position) || !Float.isFinite(extent) || extent <= 0) return 0;
        return Math.max(0, Math.min(1, position / extent));
    }

    static boolean isJpeg(byte[] data) {
        return data != null && data.length >= 4 && data.length <= MAX_JPEG_BYTES &&
                (data[0] & 255) == 255 && (data[1] & 255) == 216 &&
                (data[data.length - 2] & 255) == 255 && (data[data.length - 1] & 255) == 217;
    }

    static List<String> discoveryPrefixes(String host) {
        try {
            desktopUrl(host, 9534);
            String[] octets = host.split("\\.");
            int first = Integer.parseInt(octets[0]);
            int second = Integer.parseInt(octets[1]);
            if (first == 127) return List.of();
            String current = octets[0] + "." + octets[1] + "." + octets[2] + ".";
            List<String> prefixes = new ArrayList<>();
            prefixes.add(current);
            if (first == 192 && second == 168) {
                if (!prefixes.contains("192.168.0.")) prefixes.add("192.168.0.");
                if (!prefixes.contains("192.168.1.")) prefixes.add("192.168.1.");
            }
            return prefixes;
        } catch (Exception ignored) { return List.of(); }
    }
}
