/// 性能统计模块

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 性能指标
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// 帧率
    pub fps: f64,
    /// 端到端延迟（毫秒）
    pub latency_ms: f64,
    /// 接收的字节数
    pub bytes_received: u64,
    /// 接收的帧数
    pub frames_received: u64,
    /// 解码的帧数
    pub frames_decoded: u64,
    /// 渲染的帧数
    pub frames_rendered: u64,
    /// 丢包率
    pub packet_loss_rate: f64,
}

/// 统计收集器
pub struct StatsCollector {
    /// 统计开始时间
    start_time: Instant,
    /// 字节数
    bytes_received: Arc<AtomicU64>,
    /// 接收帧数
    frames_received: Arc<AtomicU64>,
    /// 解码帧数
    frames_decoded: Arc<AtomicU64>,
    /// 渲染帧数
    frames_rendered: Arc<AtomicU64>,
    /// 丢包数
    packets_lost: Arc<AtomicU64>,
    /// 总包数
    packets_total: Arc<AtomicU64>,
    /// FPS 计算窗口
    fps_window: Vec<Instant>,
}

impl StatsCollector {
    /// 创建新的统计收集器
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            bytes_received: Arc::new(AtomicU64::new(0)),
            frames_received: Arc::new(AtomicU64::new(0)),
            frames_decoded: Arc::new(AtomicU64::new(0)),
            frames_rendered: Arc::new(AtomicU64::new(0)),
            packets_lost: Arc::new(AtomicU64::new(0)),
            packets_total: Arc::new(AtomicU64::new(0)),
            fps_window: Vec::with_capacity(120),
        }
    }

    /// 记录接收的字节数
    pub fn record_bytes(&self, bytes: u64) {
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
    }

    /// 记录接收的帧
    pub fn record_frame_received(&self) {
        self.frames_received.fetch_add(1, Ordering::Relaxed);
    }

    /// 记录解码的帧
    pub fn record_frame_decoded(&self) {
        self.frames_decoded.fetch_add(1, Ordering::Relaxed);
    }

    /// 记录渲染的帧
    pub fn record_frame_rendered(&mut self) {
        self.frames_rendered.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        self.fps_window.push(now);

        // 清理超过 2 秒的记录
        self.fps_window.retain(|t| now.duration_since(*t) < Duration::from_secs(2));
    }

    /// 记录丢包
    pub fn record_packet_loss(&self, lost: u64) {
        self.packets_lost.fetch_add(lost, Ordering::Relaxed);
    }

    /// 记录总包数
    pub fn record_packet_total(&self, total: u64) {
        self.packets_total.fetch_add(total, Ordering::Relaxed);
    }

    /// 获取当前指标
    pub fn metrics(&self) -> PerformanceMetrics {
        let frames_received = self.frames_received.load(Ordering::Relaxed);
        let frames_decoded = self.frames_decoded.load(Ordering::Relaxed);
        let frames_rendered = self.frames_rendered.load(Ordering::Relaxed);
        let bytes_received = self.bytes_received.load(Ordering::Relaxed);
        let packets_lost = self.packets_lost.load(Ordering::Relaxed);
        let packets_total = self.packets_total.load(Ordering::Relaxed);

        // 计算 FPS
        let fps = if self.fps_window.len() > 1 {
            let duration = self.fps_window.last().unwrap().duration_since(*self.fps_window.first().unwrap());
            let count = self.fps_window.len() - 1;
            if duration.as_secs_f32() > 0.0 {
                (count as f64 / duration.as_secs_f64()).min(240.0)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // 计算丢包率
        let packet_loss_rate = if packets_total > 0 {
            (packets_lost as f64 / packets_total as f64) * 100.0
        } else {
            0.0
        };

        // 估算延迟（简单实现）
        let latency_ms = 50.0; // 默认值，实际应该从 RTCP 或时间戳计算

        PerformanceMetrics {
            fps,
            latency_ms,
            bytes_received,
            frames_received,
            frames_decoded,
            frames_rendered,
            packet_loss_rate,
        }
    }

    /// 重置统计
    pub fn reset(&mut self) {
        self.bytes_received.store(0, Ordering::Relaxed);
        self.frames_received.store(0, Ordering::Relaxed);
        self.frames_decoded.store(0, Ordering::Relaxed);
        self.frames_rendered.store(0, Ordering::Relaxed);
        self.packets_lost.store(0, Ordering::Relaxed);
        self.packets_total.store(0, Ordering::Relaxed);
        self.fps_window.clear();
        self.start_time = Instant::now();
    }

    /// 打印统计信息
    pub fn print_metrics(&self) {
        let m = self.metrics();
        tracing::info!(
            fps = m.fps,
            latency_ms = m.latency_ms,
            frames_rx = m.frames_received,
            frames_decoded = m.frames_decoded,
            frames_rendered = m.frames_rendered,
            packet_loss = format!("{:.2}%", m.packet_loss_rate),
            "performance metrics"
        );
    }
}

impl Default for StatsCollector {
    fn default() -> Self {
        Self::new()
    }
}
