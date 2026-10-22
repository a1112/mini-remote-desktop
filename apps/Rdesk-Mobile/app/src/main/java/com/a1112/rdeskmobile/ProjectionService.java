package com.a1112.rdeskmobile;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.graphics.Bitmap;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.hardware.display.DisplayManager;
import android.hardware.display.VirtualDisplay;
import android.media.Image;
import android.media.ImageReader;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.IBinder;
import android.view.WindowManager;
import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;

public final class ProjectionService extends Service {
    static final String ACTION_STOP = "com.a1112.rdeskmobile.STOP_SHARING";
    static final String EXTRA_RESULT = "result";
    static final String EXTRA_DATA = "data";
    static final String EXTRA_HOST = "host";
    static final String EXTRA_PORT = "port";
    static final String EXTRA_TOKEN = "token";
    static final String EXTRA_FINGERPRINT = "fingerprint";
    private static final String CHANNEL = "rdesk_projection";
    private static volatile boolean active;

    private MediaProjection projection;
    private VirtualDisplay virtualDisplay;
    private ImageReader reader;
    private HandlerThread imageThread;
    private LanWebSocket socket;
    private volatile boolean remoteReady;
    private long lastFrameMs;
    private boolean stopping;

    static boolean isActive() { return active; }
    @Override public IBinder onBind(Intent intent) { return null; }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || ACTION_STOP.equals(intent.getAction())) { stopSelf(); return START_NOT_STICKY; }
        if (active) return START_NOT_STICKY;
        NotificationManager notifications = getSystemService(NotificationManager.class);
        notifications.createNotificationChannel(new NotificationChannel(CHANNEL, "Rdesk 屏幕共享", NotificationManager.IMPORTANCE_LOW));
        Intent stop = new Intent(this, ProjectionService.class).setAction(ACTION_STOP);
        PendingIntent stopAction = PendingIntent.getService(this, 1, stop, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification notification = new Notification.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.ic_menu_share).setContentTitle("Rdesk 正在共享手机屏幕")
                .setContentText("点按停止可立即结束电脑控制")
                .addAction(new Notification.Action.Builder(null, "停止共享", stopAction).build()).build();
        startForeground(42, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION);
        try {
            Intent resultData = intent.getParcelableExtra(EXTRA_DATA, Intent.class);
            if (resultData == null) throw new IllegalStateException("缺少屏幕采集授权");
            MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
            projection = manager.getMediaProjection(intent.getIntExtra(EXTRA_RESULT, 0), resultData);
            if (projection == null) throw new IllegalStateException("屏幕采集授权失败");
            projection.registerCallback(new MediaProjection.Callback() {
                @Override public void onStop() { stopSelf(); }
            }, new Handler(getMainLooper()));
            Rect bounds = ((WindowManager) getSystemService(WINDOW_SERVICE)).getMaximumWindowMetrics().getBounds();
            int width = Math.min(bounds.width(), 720);
            int height = Math.max(1, Math.round(bounds.height() * (width / (float) bounds.width())));
            reader = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 2);
            imageThread = new HandlerThread("rdesk-screen-capture"); imageThread.start();
            reader.setOnImageAvailableListener(this::onImageAvailable, new Handler(imageThread.getLooper()));
            virtualDisplay = projection.createVirtualDisplay("Rdesk Mobile", width, height,
                    getResources().getDisplayMetrics().densityDpi, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                    reader.getSurface(), null, null);
            String host = intent.getStringExtra(EXTRA_HOST);
            int port = intent.getIntExtra(EXTRA_PORT, 9534);
            String token = intent.getStringExtra(EXTRA_TOKEN);
            String fingerprint = intent.getStringExtra(EXTRA_FINGERPRINT);
            socket = new LanWebSocket(Protocol.phonePublishUrl(host, port, token), fingerprint, new LanWebSocket.Listener() {
                @Override public void onText(String text) {
                    if (text.contains("\"type\":\"error\"")) { stopSelf(); return; }
                    if (text.contains("\"type\":\"ready\"")) { remoteReady = true; return; }
                    RemoteAccessibilityService.acceptControl(text);
                }
                @Override public void onBinary(byte[] bytes) {}
                @Override public void onClosed(String reason) { remoteReady = false; stopSelf(); }
            });
            active = true;
            RemoteAccessibilityService.setRemoteActive(true);
            socket.connect();
            return START_NOT_STICKY;
        } catch (Exception error) { stopSelf(); return START_NOT_STICKY; }
    }

    private void onImageAvailable(ImageReader source) {
        Image image = source.acquireLatestImage();
        if (image == null) return;
        try {
            long now = android.os.SystemClock.elapsedRealtime();
            if (!active || now - lastFrameMs < 150) return;
            lastFrameMs = now;
            Image.Plane plane = image.getPlanes()[0];
            ByteBuffer buffer = plane.getBuffer();
            int rowPixels = plane.getRowStride() / plane.getPixelStride();
            Bitmap padded = Bitmap.createBitmap(rowPixels, image.getHeight(), Bitmap.Config.ARGB_8888);
            padded.copyPixelsFromBuffer(buffer);
            Bitmap frame = Bitmap.createBitmap(padded, 0, 0, image.getWidth(), image.getHeight());
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            frame.compress(Bitmap.CompressFormat.JPEG, 65, bytes);
            frame.recycle(); padded.recycle();
            if (remoteReady && socket != null) socket.sendBinary(bytes.toByteArray());
        } catch (Exception ignored) {} finally { image.close(); }
    }

    @Override public void onDestroy() {
        if (stopping) return;
        stopping = true; active = false; remoteReady = false; RemoteAccessibilityService.setRemoteActive(false);
        if (socket != null) { socket.close(); socket = null; }
        if (reader != null) { reader.setOnImageAvailableListener(null, null); reader.close(); reader = null; }
        if (virtualDisplay != null) { virtualDisplay.release(); virtualDisplay = null; }
        if (projection != null) { projection.stop(); projection = null; }
        if (imageThread != null) { imageThread.quitSafely(); imageThread = null; }
        super.onDestroy();
    }
}
