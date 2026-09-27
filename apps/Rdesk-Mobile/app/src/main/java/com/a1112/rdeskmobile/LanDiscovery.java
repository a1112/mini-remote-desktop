package com.a1112.rdeskmobile;

import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InterfaceAddress;
import java.net.NetworkInterface;
import java.nio.charset.StandardCharsets;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.Set;
import org.json.JSONObject;

final class LanDiscovery {
    interface Listener {
        void onGateway(String address, int port, String name);
        void onFinished();
    }

    static void scan(String knownHost, boolean extended, Listener listener) {
        new Thread(() -> runScan(knownHost, extended, listener), "rdesk-lan-discovery").start();
    }

    private static void runScan(String knownHost, boolean extended, Listener listener) {
        try (DatagramSocket socket = new DatagramSocket()) {
            socket.setBroadcast(true);
            socket.setSoTimeout(350);
            Set<InetAddress> targets = new LinkedHashSet<>();
            Set<String> prefixes = new LinkedHashSet<>();
            targets.add(InetAddress.getByName("255.255.255.255"));
            try {
                Protocol.desktopUrl(knownHost, 9534);
                targets.add(InetAddress.getByName(knownHost));
            } catch (Exception ignored) {}
            Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
            while (interfaces.hasMoreElements()) {
                NetworkInterface network = interfaces.nextElement();
                if (!network.isUp() || network.isLoopback()) continue;
                for (InterfaceAddress address : network.getInterfaceAddresses()) {
                    if (address.getBroadcast() != null) {
                        targets.add(address.getBroadcast());
                        if (extended) prefixes.addAll(Protocol.discoveryPrefixes(address.getAddress().getHostAddress()));
                    }
                }
            }
            byte[] probe = "MRD_DISCOVER_V1".getBytes(StandardCharsets.US_ASCII);
            for (InetAddress target : targets) {
                try { socket.send(new DatagramPacket(probe, probe.length, target, 9535)); }
                catch (Exception ignored) {}
            }
            if (extended) {
                for (String prefix : prefixes) {
                    for (int address = 1; address <= 254; address++) {
                        try {
                            InetAddress target = InetAddress.getByName(prefix + address);
                            socket.send(new DatagramPacket(probe, probe.length, target, 9535));
                            Thread.sleep(1);
                        } catch (Exception ignored) {}
                    }
                }
            }
            Set<String> seen = new HashSet<>();
            long deadline = System.currentTimeMillis() + (extended ? 3000 : 1800);
            while (System.currentTimeMillis() < deadline) {
                byte[] buffer = new byte[512];
                DatagramPacket reply = new DatagramPacket(buffer, buffer.length);
                try { socket.receive(reply); } catch (java.net.SocketTimeoutException timeout) { continue; }
                try {
                    JSONObject json = new JSONObject(new String(reply.getData(), reply.getOffset(), reply.getLength(), StandardCharsets.UTF_8));
                    if (!"rdesk_gateway".equals(json.optString("type"))) continue;
                    String host = reply.getAddress().getHostAddress();
                    int port = json.optInt("port", -1);
                    Protocol.desktopUrl(host, port);
                    if (seen.add(host + ":" + port)) listener.onGateway(host, port, json.optString("name", "Rdesk 电脑"));
                } catch (Exception ignored) {}
            }
        } catch (Exception ignored) {}
        listener.onFinished();
    }
}
