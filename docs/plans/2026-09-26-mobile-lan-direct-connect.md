# Mobile LAN Direct Connect Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让手机和电脑在可信私有网络内无需密钥直接连接。

**Architecture:** 网关在升级 WebSocket 前检查 TCP 来源和浏览器 Origin，然后立即发送 ready。Android 与浏览器去除密钥握手，扫码设备按钮直接启动桌面会话。

**Tech Stack:** Rust axum, Java Android, HTML/JavaScript, Python WebSocket smoke tests.

---

### Task 1: Gateway policy and handshake

**Files:** `crates/mrd-mobile-gateway/src/lib.rs`, `apps/mrd-mobile-gateway/src/main.rs`, `apps/mrd-service/src/web_bridge.rs`

1. Add failing tests for RFC1918/loopback acceptance, public rejection, and Origin/Host matching.
2. Run `cargo test -p mrd-mobile-gateway` with `CARGO_TARGET_DIR=D:\codex-mrd-mobile-target` and confirm failure.
3. Add ConnectInfo to both axum servers; reject other sources before upgrade. Remove token state/auth handshake and send ready on connection.
4. Run gateway tests, clippy, and mrd-service check.

### Task 2: Android and browser clients

**Files:** `apps/Rdesk-Mobile/app/src/main/java/com/a1112/rdeskmobile/{MainActivity,LanWebSocket,ProjectionService}.java`, `crates/mrd-mobile-gateway/src/mobile_phone_page.html`

1. Remove Android secret field, validation and auth message; make discovered device selection connect immediately.
2. Remove browser secret field and auth message; connect page when loaded.
3. Build Android unit tests and APK.

### Task 3: End-to-end verification and rollout

**Files:** `tests/mobile-gateway/smoke.py`, `apps/Rdesk-Mobile/README.md`

1. Update smoke checks for direct connection and forged Origin rejection.
2. Restart the interactive Windows gateway and run smoke tests.
3. Install updated APK on Xiaomi 14 Pro and inspect UI, discovery, and connection status.
