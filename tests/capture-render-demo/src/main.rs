#![allow(deprecated, unexpected_cfgs)]

use anyhow::{anyhow, bail, Context, Result};
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat};
use mrd_render::RenderFrame;
use std::env;

#[derive(Debug, Clone)]
struct DemoOptions {
    width: usize,
    height: usize,
    fps: u32,
    duration_ms: u64,
    continuous: bool,
    window_id: Option<u32>,
    list_windows: bool,
}

impl Default for DemoOptions {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            fps: 60,
            duration_ms: 15_000,
            continuous: false,
            window_id: None,
            list_windows: false,
        }
    }
}

impl DemoOptions {
    fn parse() -> Result<Self> {
        let mut options = Self::default();
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--width" => {
                    options.width = parse_next(&mut args, "--width")?;
                }
                "--height" => {
                    options.height = parse_next(&mut args, "--height")?;
                }
                "--fps" => {
                    options.fps = parse_next(&mut args, "--fps")?;
                }
                "--duration-ms" => {
                    options.duration_ms = parse_next(&mut args, "--duration-ms")?;
                }
                "--continuous" => {
                    options.continuous = true;
                }
                "--window-id" => {
                    options.window_id = Some(parse_next(&mut args, "--window-id")?);
                }
                "--list-windows" => {
                    options.list_windows = true;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }

        if options.width < 2 || options.height < 2 {
            bail!("width and height must be at least 2 pixels");
        }
        if options.fps == 0 {
            bail!("fps must be greater than 0");
        }
        if options.duration_ms == 0 {
            options.continuous = true;
        }

        Ok(options)
    }
}

fn parse_next<T>(args: &mut impl Iterator<Item = String>, name: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = args
        .next()
        .ok_or_else(|| anyhow!("{name} requires a value"))?;
    value
        .parse::<T>()
        .map_err(|error| anyhow!("invalid {name} value {value:?}: {error}"))
}

fn print_help() {
    println!(
        "Capture to render demo\n\n\
         Usage:\n  \
         cargo run --manifest-path tests/capture-render-demo/Cargo.toml -- [options]\n\n\
         Options:\n  \
         --width <px>         Capture/render width, default 1280\n  \
         --height <px>        Capture/render height, default 720\n  \
         --fps <n>            Target presentation rate, default 60\n  \
         --duration-ms <ms>   Run duration, default 15000\n  \
         --continuous         Run until the render window is closed or the process is stopped\n  \
         --window-id <id>     macOS ScreenCaptureKit window ID\n  \
         --list-windows       List macOS capturable windows\n"
    );
}

fn main() -> Result<()> {
    let options = DemoOptions::parse()?;
    run(options)
}

#[cfg(target_os = "macos")]
fn run(options: DemoOptions) -> Result<()> {
    use cocoa::{
        appkit::{
            NSApp, NSApplication, NSApplicationActivateIgnoringOtherApps,
            NSApplicationActivationPolicyRegular, NSRunningApplication,
        },
        base::nil,
        foundation::NSAutoreleasePool,
    };
    use mrd_capture_macos::{enumerate_window_capture_targets, MacosScreenCapture};
    use mrd_pipeline_core::FrameCapture;
    use mrd_render::{RenderTarget, RendererFactory};
    use mrd_render_macos::MacosRendererFactory;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    if options.list_windows {
        for target in enumerate_window_capture_targets()? {
            println!(
                "{:>10}  {:>5}x{:<5}  pid={:<8}  app={}  title={}",
                target.window_id,
                target.width,
                target.height,
                target.process_id,
                target.app_name,
                target.title
            );
        }
        return Ok(());
    }

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyRegular);
        app.finishLaunching();

        let window = create_macos_window(options.width, options.height)?;
        let ns_view = content_view(window)?;

        let mut capture = match options.window_id {
            Some(window_id) => MacosScreenCapture::new_window(window_id)
                .with_context(|| format!("create macOS window capture for id {window_id}"))?,
            None => MacosScreenCapture::new_primary().context("create macOS primary capture")?,
        };
        capture.set_target_dimensions(options.width, options.height);

        let mut renderer = MacosRendererFactory
            .create()
            .map_err(|error| anyhow!("create Metal renderer failed: {error}"))?;
        renderer
            .attach_target(RenderTarget::WindowHandle(ns_view as isize))
            .map_err(|error| anyhow!("attach Metal renderer failed: {error}"))?;

        let current_app = NSRunningApplication::currentApplication(nil);
        current_app.activateWithOptions_(NSApplicationActivateIgnoringOtherApps);

        println!(
            "capture={} target={}x{} fps={} duration={} renderer=metal",
            capture.backend_name(),
            options.width,
            options.height,
            options.fps,
            if options.continuous {
                "continuous".to_string()
            } else {
                format!("{}ms", options.duration_ms)
            }
        );

        let frame_interval = Duration::from_secs_f64(1.0 / options.fps as f64);
        let started = Instant::now();
        let mut frames = 0_u64;
        let mut last_report = Instant::now();
        let mut last_report_frames = 0_u64;

        while options.continuous || started.elapsed() < Duration::from_millis(options.duration_ms) {
            let frame_started = Instant::now();
            let frame = capture.capture_frame().context("capture frame")?;
            let render_frame = captured_frame_into_render_frame(frame)?;
            renderer
                .upload_frame(render_frame)
                .map_err(|error| anyhow!("upload frame to renderer failed: {error}"))?;
            frames = frames.saturating_add(1);

            if !pump_macos_events(app, window) {
                break;
            }

            let elapsed = frame_started.elapsed();
            if elapsed < frame_interval {
                thread::sleep(frame_interval - elapsed);
            }

            let report_elapsed = last_report.elapsed();
            if report_elapsed >= Duration::from_secs(1) {
                let report_frames = frames.saturating_sub(last_report_frames);
                let fps = report_frames as f64 / report_elapsed.as_secs_f64();
                println!(
                    "frames={} fps={:.1} last_frame_ms={:.2}",
                    frames,
                    fps,
                    frame_started.elapsed().as_secs_f64() * 1000.0
                );
                last_report = Instant::now();
                last_report_frames = frames;
            }
        }

        close_macos_window(window);
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run(_options: DemoOptions) -> Result<()> {
    bail!("capture-render-demo currently implements the direct visual path on macOS")
}

fn captured_frame_into_render_frame(frame: CapturedFrame) -> Result<RenderFrame> {
    let expected_pixels = frame
        .width
        .checked_mul(frame.height)
        .ok_or_else(|| anyhow!("frame size overflow"))?;

    match frame.pixel_format {
        FramePixelFormat::Bgra32 => {
            let expected = expected_pixels
                .checked_mul(4)
                .ok_or_else(|| anyhow!("BGRA frame size overflow"))?;
            if frame.data.len() != expected {
                bail!(
                    "BGRA frame byte length mismatch: expected {expected}, got {}",
                    frame.data.len()
                );
            }
            Ok(RenderFrame::from_bgra32(
                frame.width,
                frame.height,
                frame.data,
            ))
        }
        FramePixelFormat::Rgb24 => {
            let expected = expected_pixels
                .checked_mul(3)
                .ok_or_else(|| anyhow!("RGB frame size overflow"))?;
            if frame.data.len() != expected {
                bail!(
                    "RGB frame byte length mismatch: expected {expected}, got {}",
                    frame.data.len()
                );
            }
            Ok(RenderFrame::from_rgb24(
                frame.width,
                frame.height,
                frame.data,
            ))
        }
        FramePixelFormat::Rgba32 => {
            let expected = expected_pixels
                .checked_mul(4)
                .ok_or_else(|| anyhow!("RGBA frame size overflow"))?;
            if frame.data.len() != expected {
                bail!(
                    "RGBA frame byte length mismatch: expected {expected}, got {}",
                    frame.data.len()
                );
            }
            let mut bgra = frame.data;
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            Ok(RenderFrame::from_bgra32(frame.width, frame.height, bgra))
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn create_macos_window(width: usize, height: usize) -> Result<cocoa::base::id> {
    use cocoa::{
        appkit::{NSBackingStoreBuffered, NSView, NSWindow, NSWindowStyleMask},
        base::{id, nil, NO, YES},
        foundation::{NSPoint, NSRect, NSSize, NSString},
    };
    use objc::{msg_send, sel, sel_impl};

    let frame = NSRect::new(
        NSPoint::new(80.0, 80.0),
        NSSize::new(width as f64, height as f64),
    );
    let style = NSWindowStyleMask::NSTitledWindowMask
        | NSWindowStyleMask::NSClosableWindowMask
        | NSWindowStyleMask::NSMiniaturizableWindowMask
        | NSWindowStyleMask::NSResizableWindowMask;
    let window: id = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
        frame,
        style,
        NSBackingStoreBuffered,
        NO,
    );
    if window == nil {
        bail!("create macOS render window failed");
    }

    let view_frame = NSRect::new(NSPoint::new(0.0, 0.0), frame.size);
    let view: id = NSView::alloc(nil).initWithFrame_(view_frame);
    if view == nil {
        let _: () = msg_send![window, release];
        bail!("create macOS render view failed");
    }

    let _: () = msg_send![window, setReleasedWhenClosed: NO];
    view.setWantsLayer(YES);
    window.setContentView_(view);
    let title = NSString::alloc(nil).init_str("MRD Capture -> Metal Render Demo");
    window.setTitle_(title);
    let _: () = msg_send![title, release];
    window.center();
    window.makeKeyAndOrderFront_(nil);

    Ok(window)
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn content_view(window: cocoa::base::id) -> Result<cocoa::base::id> {
    use cocoa::base::{id, nil};
    use objc::{msg_send, sel, sel_impl};

    let view: id = msg_send![window, contentView];
    if view == nil {
        bail!("macOS render window has no content view");
    }
    Ok(view)
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn pump_macos_events(app: cocoa::base::id, window: cocoa::base::id) -> bool {
    use cocoa::{
        appkit::NSApplication,
        base::{id, nil, YES},
        foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSUInteger},
    };
    use objc::{msg_send, sel, sel_impl};

    let pool = NSAutoreleasePool::new(nil);
    loop {
        let event: id = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
            usize::MAX as NSUInteger,
            nil,
            NSDefaultRunLoopMode,
            YES,
        );
        if event == nil {
            break;
        }
        app.sendEvent_(event);
    }
    let _: () = msg_send![app, updateWindows];
    let visible: bool = msg_send![window, isVisible];
    pool.drain();
    visible
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn close_macos_window(window: cocoa::base::id) {
    use cocoa::base::nil;
    use objc::{msg_send, sel, sel_impl};

    let _: () = msg_send![window, orderOut: nil];
    let _: () = msg_send![window, close];
    let _: () = msg_send![window, release];
}
