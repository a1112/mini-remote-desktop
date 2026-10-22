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
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.Arrays;
import java.util.Base64;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509TrustManager;

final class LanWebSocket {
    interface Listener {
        void onText(String text);
        void onBinary(byte[] bytes);
        void onClosed(String reason);
    }

    private final String url;
    private final String certificateFingerprint;
    private final Listener listener;
    private final SecureRandom random = new SecureRandom();
    private volatile Socket socket;
    private volatile OutputStream output;
    private volatile boolean closed;

    LanWebSocket(String url, String certificateFingerprint, Listener listener) {
        this.url = url; this.certificateFingerprint = certificateFingerprint; this.listener = listener;
    }

    void connect() {
        new Thread(this::run, "rdesk-websocket").start();
    }

    private void run() {
        String reason = "连接已断开";
        try {
            URI uri = URI.create(url);
            if (!"wss".equals(uri.getScheme())) throw new IllegalArgumentException("仅支持加密 WebSocket");
            if (!Protocol.isCertificateFingerprintValid(certificateFingerprint)) throw new IllegalArgumentException("缺少网关证书指纹");
            SSLSocket connection = openPinnedTlsSocket(uri); socket = connection;
            connection.connect(new InetSocketAddress(uri.getHost(), uri.getPort()), 5000);
            connection.startHandshake();
            connection.setTcpNoDelay(true);
            InputStream input = connection.getInputStream(); output = connection.getOutputStream();
            byte[] nonce = new byte[16]; random.nextBytes(nonce);
            String key = Base64.getEncoder().encodeToString(nonce);
            String target = uri.getRawPath();
            if (uri.getRawQuery() != null && !uri.getRawQuery().isEmpty()) target += "?" + uri.getRawQuery();
            String request = "GET " + target + " HTTP/1.1\r\nHost: " + uri.getHost() + ":" + uri.getPort() +
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

    private SSLSocket openPinnedTlsSocket(URI uri) throws Exception {
        TrustManager[] trustManagers = new TrustManager[] { new X509TrustManager() {
            @Override public void checkClientTrusted(X509Certificate[] chain, String authType) throws CertificateException {}
            @Override public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException {
                if (chain == null || chain.length == 0) throw new CertificateException("网关未提供证书");
                try {
                    String actual = hex(MessageDigest.getInstance("SHA-256").digest(chain[0].getEncoded()));
                    if (!actual.equalsIgnoreCase(certificateFingerprint)) throw new CertificateException("网关证书指纹不匹配");
                } catch (CertificateException error) { throw error; }
                catch (Exception error) { throw new CertificateException("无法验证网关证书", error); }
            }
            @Override public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
        }};
        SSLContext context = SSLContext.getInstance("TLS");
        context.init(null, trustManagers, random);
        return (SSLSocket) context.getSocketFactory().createSocket();
    }

    private static String hex(byte[] bytes) {
        StringBuilder value = new StringBuilder(bytes.length * 2);
        for (byte item : bytes) value.append(String.format("%02x", item & 0xff));
        return value.toString();
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
