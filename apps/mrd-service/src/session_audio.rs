//! System audio travels only on the already authenticated session's QUIC
//! connection and only while its independently approved audio.listen lease lives.
use crate::AppState;
use mrd_ipc::{IpcResponse, SessionAudioSnapshot};
use mrd_proto::SessionId;
use std::{collections::HashMap, sync::Arc};

pub const CAPABILITY: &str = "audio.system.pcm16.v1";

#[derive(Default)]
pub struct AudioRegistry {
    sessions: HashMap<SessionId, AudioEntry>,
}
struct AudioEntry {
    snapshot: SessionAudioSnapshot,
    #[cfg(target_os = "macos")]
    playback: Option<mrd_audio::macos::AudioPlayback>,
}
impl AudioRegistry {
    fn start(&mut self, session: &SessionId) {
        self.sessions.insert(
            session.clone(),
            AudioEntry {
                snapshot: empty_snapshot(session, "starting"),
                #[cfg(target_os = "macos")]
                playback: None,
            },
        );
    }
    pub fn remove(&mut self, session: &SessionId) {
        self.sessions.remove(session);
    }
    pub fn stop_all(&mut self) {
        self.sessions.clear();
    }
    fn fail(&mut self, session: &SessionId, error: String) {
        if let Some(entry) = self.sessions.get_mut(session) {
            entry.snapshot.status = "failed".into();
            entry.snapshot.last_error = Some(error);
            #[cfg(target_os = "macos")]
            {
                entry.playback = None;
            }
        }
    }
    fn snapshot(&self, session: &SessionId) -> SessionAudioSnapshot {
        let Some(entry) = self.sessions.get(session) else {
            return empty_snapshot(session, "inactive");
        };
        #[allow(unused_mut)]
        let mut snapshot = entry.snapshot.clone();
        #[cfg(target_os = "macos")]
        if let Some(playback) = &entry.playback {
            if let Ok(buffer) = playback.buffer.lock() {
                snapshot.packets = buffer.received_packets;
                snapshot.dropped_packets = buffer.dropped_packets;
                snapshot.buffered_frames = buffer.buffered_frames();
                snapshot.underrun_packets = buffer.underrun_packets;
            }
            if let Ok(error) = playback.error.lock() {
                if let Some(error) = error.as_ref() {
                    snapshot.status = "failed".into();
                    snapshot.last_error = Some(error.clone());
                }
            }
        }
        snapshot
    }
}
fn empty_snapshot(session: &SessionId, status: &str) -> SessionAudioSnapshot {
    SessionAudioSnapshot {
        session_id: session.clone(),
        status: status.into(),
        muted: false,
        packets: 0,
        dropped_packets: 0,
        buffered_frames: 0,
        underrun_packets: 0,
        sample_rate: mrd_audio::SAMPLE_RATE,
        channels: mrd_audio::CHANNELS as u16,
        last_error: None,
    }
}

pub async fn snapshot(
    state: &Arc<AppState>,
    session: SessionId,
    muted: Option<bool>,
) -> IpcResponse {
    let guard = state.authorization_security_gate.lock().await;
    if state
        .session_authorizations
        .audio_binding(&session)
        .await
        .is_none()
        || !state.security_is_healthy()
    {
        state.session_audio.lock().await.remove(&session);
        return IpcResponse::SessionAudioSnapshot {
            snapshot: empty_snapshot(&session, "not_authorized"),
        };
    }
    let mut registry = state.session_audio.lock().await;
    if let (Some(muted), Some(entry)) = (muted, registry.sessions.get_mut(&session)) {
        entry.snapshot.muted = muted;
        #[cfg(target_os = "macos")]
        if let Some(playback) = &entry.playback {
            playback
                .muted
                .store(muted, std::sync::atomic::Ordering::Release);
        }
    }
    let snapshot = registry.snapshot(&session);
    drop(guard);
    IpcResponse::SessionAudioSnapshot { snapshot }
}

#[cfg(target_os = "macos")]
pub async fn spawn_sender(
    state: Arc<AppState>,
    session: SessionId,
    endpoint: mrd_transport_quic_quinn::QuinnDatagramEndpoint,
) -> Option<tokio::task::JoinHandle<()>> {
    use anyhow::{Context, Result};
    use mrd_ipc::RemotePermissionScope;
    use std::time::Duration;
    let binding = state.session_authorizations.audio_binding(&session).await?;
    let lease = state
        .session_authorizations
        .acquire_scope_lease(&session, RemotePermissionScope::AudioListen)
        .await?;
    state.session_audio.lock().await.start(&session);
    Some(tokio::spawn(async move {
        let run = async {
            anyhow::ensure!(
                endpoint.max_datagram_size().unwrap_or(0) >= mrd_audio::PACKET_BYTES,
                "QUIC MTU cannot carry system audio"
            );
            let capture = tokio::task::spawn_blocking(move || {
                mrd_audio::macos::SystemAudioCapture::start(binding.0)
            })
            .await??;
            if let Some(entry) = state.session_audio.lock().await.sessions.get_mut(&session) {
                entry.snapshot.status = "capturing".into();
            }
            let mut last_sample = std::time::Instant::now();
            loop {
                match capture.packets.try_recv() {
                    Ok(captured) => {
                        last_sample = std::time::Instant::now();
                        if captured.captured_at.elapsed() > Duration::from_millis(50) {
                            continue;
                        }
                        let _guard = state.authorization_security_gate.lock().await;
                        if !state.security_is_healthy()
                            || state.session_authorizations.audio_binding(&session).await
                                != Some(binding)
                        {
                            break;
                        }
                        let bytes = captured.packet.encode()?;
                        endpoint
                            .send_datagram(bytes.into())
                            .context("send system audio")?;
                        if let Some(entry) =
                            state.session_audio.lock().await.sessions.get_mut(&session)
                        {
                            entry.snapshot.packets += 1;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        anyhow::ensure!(last_sample.elapsed() < Duration::from_secs(5),
                            "system audio capture produced no samples for five seconds; check Screen Recording permission and the audio device");
                        tokio::time::sleep(Duration::from_millis(2)).await
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        anyhow::bail!("system audio capture stopped")
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        let result: Result<()> = tokio::select! {
            biased;
            _ = lease.wait_until_invalid() => Ok(()),
            result = run => result,
        };
        match result {
            Ok(()) => state.session_audio.lock().await.remove(&session),
            Err(error) => state
                .session_audio
                .lock()
                .await
                .fail(&session, error.to_string()),
        }
    }))
}

#[cfg(target_os = "macos")]
pub async fn spawn_receiver(
    state: Arc<AppState>,
    session: SessionId,
) -> Option<(
    tokio::sync::mpsc::Sender<bytes::Bytes>,
    tokio::task::JoinHandle<()>,
)> {
    use mrd_ipc::RemotePermissionScope;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let binding = state.session_authorizations.audio_binding(&session).await?;
    let lease = state
        .session_authorizations
        .acquire_scope_lease(&session, RemotePermissionScope::AudioListen)
        .await?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(8);
    state.session_audio.lock().await.start(&session);
    let task = tokio::spawn(async move {
        let run = async {
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
            let deadline = Instant::now() + Duration::from_millis(binding.1.saturating_sub(now));
            let playback = tokio::task::spawn_blocking(move || {
                mrd_audio::macos::AudioPlayback::start(deadline)
            })
            .await??;
            let buffer = playback.buffer.clone();
            {
                let mut registry = state.session_audio.lock().await;
                let Some(entry) = registry.sessions.get_mut(&session) else {
                    return Ok::<_, anyhow::Error>(());
                };
                playback
                    .muted
                    .store(entry.snapshot.muted, std::sync::atomic::Ordering::Release);
                entry.snapshot.status = "listening".into();
                entry.playback = Some(playback);
            }
            while let Some(bytes) = rx.recv().await {
                let _guard = state.authorization_security_gate.lock().await;
                if !state.security_is_healthy()
                    || state.session_authorizations.audio_binding(&session).await != Some(binding)
                {
                    break;
                }
                match mrd_audio::AudioPacket::decode(&bytes, binding.0) {
                    Ok(packet) => {
                        buffer
                            .lock()
                            .map_err(|_| anyhow::anyhow!("audio buffer poisoned"))?
                            .push(packet);
                        if let Some(entry) =
                            state.session_audio.lock().await.sessions.get_mut(&session)
                        {
                            entry.snapshot.status = "playing".into();
                        }
                    }
                    Err(_) => {
                        if let Ok(mut buffer) = buffer.lock() {
                            buffer.dropped_packets += 1;
                        }
                    }
                }
            }
            Ok(())
        };
        let result = tokio::select! {
            biased;
            _ = lease.wait_until_invalid() => Ok(()),
            result = run => result,
        };
        match result {
            Ok(()) => state.session_audio.lock().await.remove(&session),
            Err(error) => state
                .session_audio
                .lock()
                .await
                .fail(&session, error.to_string()),
        }
    });
    Some((tx, task))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_authorization::{
        VerifiedIncomingAuthorizationRequest, VerifiedSessionGrant,
    };
    use mrd_ipc::{
        RemoteAccessMode, RemoteAuthorizationState, RemoteFailure, RemotePermissionScope,
        RemoteReasonCode,
    };

    async fn granted_state(name: &str, audio: bool) -> (Arc<AppState>, SessionId) {
        let state = Arc::new(AppState::default());
        let session = SessionId(name.into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let scopes = if audio {
            vec![
                RemotePermissionScope::ScreenView,
                RemotePermissionScope::AudioListen,
            ]
        } else {
            vec![RemotePermissionScope::ScreenView]
        };
        state
            .session_authorizations
            .begin_outgoing(VerifiedIncomingAuthorizationRequest {
                session_id: session.clone(),
                peer_device_id: mrd_proto::DeviceId("audio-peer".into()),
                peer_key_id: "audio-peer-key".into(),
                peer_key_epoch: 1,
                access_mode: RemoteAccessMode::Attended,
                requested_scopes: scopes.clone(),
                peer_permission_ceiling: scopes.clone(),
                machine_permission_ceiling: scopes.clone(),
                runtime_capabilities: scopes.clone(),
                transport_kind: "quic".into(),
                request_nonce: [3; 16],
                created_at_ms: now - 10,
                expires_at_ms: now + 60_000,
            })
            .await
            .unwrap();
        state
            .session_authorizations
            .install_verified_grant(
                VerifiedSessionGrant {
                    grant_id: format!("sha256:{}", "ab".repeat(32)),
                    session_id: session.clone(),
                    granted_scopes: scopes,
                    issued_at_ms: now - 5,
                    expires_at_ms: now + 60_000,
                    policy_revision: 1,
                    route_constraint: "quic".into(),
                    transport_fingerprint_sha256: [9; 32],
                },
                now,
            )
            .await
            .unwrap();
        (state, session)
    }

    async fn revoke(state: &AppState, session: &SessionId) {
        let _gate = state.authorization_security_gate.lock().await;
        state
            .session_authorizations
            .record_failure(
                session,
                RemoteAuthorizationState::Revoked,
                RemoteFailure {
                    code: RemoteReasonCode::PolicyChanged,
                    message: "test revocation".into(),
                    suggested_action: None,
                },
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn screen_grants_do_not_enable_audio_and_revocation_invalidates_audio_lease() {
        let (screen, screen_session) = granted_state("screen-only", false).await;
        assert!(screen
            .session_authorizations
            .audio_binding(&screen_session)
            .await
            .is_none());
        let (state, session) = granted_state("audio-granted", true).await;
        let binding = state
            .session_authorizations
            .audio_binding(&session)
            .await
            .unwrap();
        let (other, other_session) = granted_state("another-session", true).await;
        assert_ne!(
            binding.0,
            other
                .session_authorizations
                .audio_binding(&other_session)
                .await
                .unwrap()
                .0
        );
        let lease = state
            .session_authorizations
            .acquire_scope_lease(&session, RemotePermissionScope::AudioListen)
            .await
            .unwrap();
        revoke(&state, &session).await;
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            lease.wait_until_invalid(),
        )
        .await
        .unwrap();
        assert!(state
            .session_authorizations
            .audio_binding(&session)
            .await
            .is_none());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires native CoreAudio output on an unlocked Mac"]
    async fn native_receiver_mute_and_revocation_stop_playback() {
        use std::sync::atomic::Ordering;
        use std::time::Duration;
        let (state, session) = granted_state("native-audio-revoke", true).await;
        let binding = state
            .session_authorizations
            .audio_binding(&session)
            .await
            .unwrap();
        let (tx, task) = spawn_receiver(state.clone(), session.clone())
            .await
            .unwrap();
        for sequence in 0..12 {
            tx.send(
                mrd_audio::AudioPacket {
                    stream_id: binding.0,
                    sequence,
                    samples: vec![0; mrd_audio::SAMPLES_PER_PACKET],
                }
                .encode()
                .unwrap()
                .into(),
            )
            .await
            .unwrap();
        }
        let active = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(entry) = state.session_audio.lock().await.sessions.get(&session) {
                    if let Some(playback) = &entry.playback {
                        if playback.buffer.lock().unwrap().received_packets == 12 {
                            break playback.active.clone();
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(snapshot(&state, session.clone(), Some(true)).await,
            IpcResponse::SessionAudioSnapshot { snapshot } if snapshot.muted && snapshot.packets == 12)
        );
        assert!(state
            .session_audio
            .lock()
            .await
            .sessions
            .get(&session)
            .unwrap()
            .playback
            .as_ref()
            .unwrap()
            .muted
            .load(Ordering::Acquire));
        revoke(&state, &session).await;
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(!active.load(Ordering::Acquire));
        assert!(state.session_audio.lock().await.sessions.is_empty());
        assert!(tx.send(bytes::Bytes::new()).await.is_err());
        println!("native receiver: 12 authenticated packets, mute applied, revoke stopped output and closed queue");
    }
    #[tokio::test]
    async fn audio_is_never_available_without_an_audio_grant() {
        let state = Arc::new(AppState::default());
        let session = SessionId("unauthorized-audio".into());
        state.session_audio.lock().await.start(&session);
        let response = snapshot(&state, session.clone(), Some(false)).await;
        assert!(
            matches!(response, IpcResponse::SessionAudioSnapshot { snapshot } if snapshot.status == "not_authorized")
        );
        assert!(state.session_audio.lock().await.sessions.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "plays a test sound; requires native ScreenCaptureKit and CoreAudio permissions"]
    async fn native_system_audio_crosses_quic_mux_and_stops_on_revoke() {
        use crate::transports::{quic::QuicTransportMux, TransportMuxConfig};
        use std::{
            process::{Command, Stdio},
            sync::atomic::{AtomicU64, Ordering},
            time::Duration,
        };
        let (sender, session) = granted_state("native-quic-audio", true).await;
        let (receiver, _) = granted_state("native-quic-audio", true).await;
        let binding = sender
            .session_authorizations
            .audio_binding(&session)
            .await
            .unwrap()
            .0;
        let pair = mrd_transport_quic_quinn::QuinnDatagramPair::loopback()
            .await
            .unwrap();
        let mux =
            QuicTransportMux::new(session.clone(), TransportMuxConfig::default(), pair.client);
        let (tx, receiver_task) = spawn_receiver(receiver.clone(), session.clone())
            .await
            .unwrap();
        let peak = Arc::new(AtomicU64::new(0));
        let observed_peak = peak.clone();
        let router = tokio::spawn(async move {
            while let Some(bytes) = mux.recv_passthrough_datagram().await {
                let packet = mrd_audio::AudioPacket::decode(&bytes, binding).unwrap();
                observed_peak.fetch_max(
                    packet
                        .samples
                        .iter()
                        .map(|s| i32::from(*s).unsigned_abs() as u64)
                        .max()
                        .unwrap_or(0),
                    Ordering::Relaxed,
                );
                if tx.send(bytes).await.is_err() {
                    break;
                }
            }
        });
        let sender_task = spawn_sender(sender.clone(), session.clone(), pair.server)
            .await
            .unwrap();
        // Give native capture time to start before the finite test sound.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if sender.session_audio.lock().await.snapshot(&session).status == "capturing" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let mut sound = Command::new("afplay")
            .arg("/System/Library/Sounds/Glass.aiff")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = sound.kill();
        let _ = sound.wait();
        let sent = sender.session_audio.lock().await.snapshot(&session);
        let received = receiver.session_audio.lock().await.snapshot(&session);
        revoke(&sender, &session).await;
        revoke(&receiver, &session).await;
        tokio::time::timeout(Duration::from_secs(1), sender_task)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), receiver_task)
            .await
            .unwrap()
            .unwrap();
        router.abort();
        println!(
            "native audio over QUIC mux: sent={}, received={}, peak={}, dropped={}, underruns={}",
            sent.packets,
            received.packets,
            peak.load(Ordering::Relaxed),
            received.dropped_packets,
            received.underrun_packets
        );
        assert!(sent.packets > 100 && received.packets > 100);
        assert!(peak.load(Ordering::Relaxed) > 100);
        assert_eq!(received.status, "playing");
        assert!(sender.session_audio.lock().await.sessions.is_empty());
        assert!(receiver.session_audio.lock().await.sessions.is_empty());
    }
}
