package com.a1112.rdeskmobile;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Arrays;
import java.util.Base64;

final class LanWebSocket {
    interface Listener {
        void onText(String text);
        void onBinary(byte[] bytes);
        void onClosed(String reason);
    }

    private final String url;
    private final Listener listener;
    private final SecureRandom random = new SecureRandom();
    private volatile Socket socket;
    private volatile OutputStream output;
    private volatile boolean closed;

    LanWebSocket(String url, Listener listener) {
        this.url = url; this.listener = listener;
    }

    void connect() {
        new Thread(this::run, "rdesk-websocket").start();
    }

    private void run() {
        String reason = "连接已断开";
        try {
            URI uri = URI.create(url);
            if (!"ws".equals(uri.getScheme())) throw new IllegalArgumentException("仅支持局域网 WebSocket");
            Socket connection = new Socket(); socket = connection;
            connection.connect(new InetSocketAddress(uri.getHost(), uri.getPort()), 5000);
            connection.setTcpNoDelay(true);
            InputStream input = connection.getInputStream(); output = connection.getOutputStream();
            byte[] nonce = new byte[16]; random.nextBytes(nonce);
            String key = Base64.getEncoder().encodeToString(nonce);
            String request = "GET " + uri.getRawPath() + " HTTP/1.1\r\nHost: " + uri.getHost() + ":" + uri.getPort() +
                    "\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: " + key +
                    "\r\nSec-WebSocket-Version: 13\r\n\r\n";
            output.write(request.getBytes(StandardCharsets.US_ASCII)); output.flush();
            String status = readLine(input);
            if (!status.startsWith("HTTP/1.1 101") && !status.startsWith("HTTP/1.0 101")) throw new IllegalStateException("网关拒绝 WebSocket 连接: " + status);
            String accept = null;
            for (String line; !(line = readLine(input)).isEmpty();) {
                if (line.toLowerCase().startsWith("sec-websocket-accept:")) accept = line.substring(line.indexOf(':') + 1).trim();
            }
            String expected = Base64.getEncoder().encodeToString(MessageDigest.getInstance("SHA-1")
                    .digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").getBytes(StandardCharsets.US_ASCII)));
            if (!expected.equals(accept)) throw new IllegalStateException("网关握手验证失败");
            while (!closed) {
                int first = input.read(), second = input.read();
                if (first < 0 || second < 0) break;
                int opcode = first & 0x0f;
                if ((first & 0x80) == 0) throw new IllegalStateException("不支持分片帧");
                long length = second & 0x7f;
                if (length == 126) length = ((long) readByte(input) << 8) | readByte(input);
                if (length == 127) {
                    length = 0;
                    for (int i = 0; i < 8; i++) length = (length << 8) | readByte(input);
                }
                if (length > Protocol.MAX_JPEG_BYTES) throw new IllegalStateException("帧超过大小限制");
                byte[] mask = null;
                if ((second & 0x80) != 0) { mask = new byte[4]; readFully(input, mask); }
                byte[] data = new byte[(int) length]; readFully(input, data);
                if (mask != null) for (int i = 0; i < data.length; i++) data[i] ^= mask[i & 3];
                if (opcode == 8) break;
                if (opcode == 9) { sendFrame(10, data); continue; }
                if (opcode == 1 && data.length <= 8192) listener.onText(new String(data, StandardCharsets.UTF_8));
                if (opcode == 2 && Protocol.isJpeg(data)) listener.onBinary(data);
            }
        } catch (Exception error) { reason = error.getMessage() == null ? "网络连接失败" : error.getMessage(); }
        finally { close(); listener.onClosed(reason); }
    }

    void sendText(String text) { sendFrame(1, text.getBytes(StandardCharsets.UTF_8)); }
    void sendBinary(byte[] bytes) { if (Protocol.isJpeg(bytes)) sendFrame(2, bytes); }

    private synchronized void sendFrame(int opcode, byte[] bytes) {
        if (closed || output == null) return;
        try {
            output.write(0x80 | opcode);
            if (bytes.length < 126) output.write(0x80 | bytes.length);
            else if (bytes.length <= 65535) { output.write(0x80 | 126); output.write(bytes.length >> 8); output.write(bytes.length); }
            else { output.write(0x80 | 127); output.write(new byte[] {0,0,0,0,(byte)(bytes.length >> 24),(byte)(bytes.length >> 16),(byte)(bytes.length >> 8),(byte)bytes.length}); }
            byte[] mask = new byte[4]; random.nextBytes(mask); output.write(mask);
            byte[] masked = Arrays.copyOf(bytes, bytes.length);
            for (int i = 0; i < masked.length; i++) masked[i] ^= mask[i & 3];
            output.write(masked); output.flush();
        } catch (Exception ignored) { close(); }
    }

    void close() {
        closed = true;
        Socket connection = socket;
        if (connection != null) try { connection.close(); } catch (Exception ignored) {}
    }

    private static int readByte(InputStream input) throws Exception {
        int value = input.read(); if (value < 0) throw new IllegalStateException("连接已关闭"); return value;
    }

    private static void readFully(InputStream input, byte[] data) throws Exception {
        int offset = 0;
        while (offset < data.length) { int count = input.read(data, offset, data.length - offset); if (count < 0) throw new IllegalStateException("连接已关闭"); offset += count; }
    }

    private static String readLine(InputStream input) throws Exception {
        ByteArrayOutputStream line = new ByteArrayOutputStream();
        for (int value; (value = readByte(input)) != '\n';) {
            if (value != '\r') line.write(value);
            if (line.size() > 8192) throw new IllegalStateException("握手头过长");
        }
        return line.toString(StandardCharsets.US_ASCII.name());
    }
}
