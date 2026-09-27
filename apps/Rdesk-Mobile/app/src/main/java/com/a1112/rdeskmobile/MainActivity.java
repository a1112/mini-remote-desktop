package com.a1112.rdeskmobile;

import android.app.Activity;
import android.app.NotificationManager;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.RectF;
import android.media.projection.MediaProjectionManager;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.WindowInsets;
import android.widget.Button;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import org.json.JSONObject;

public final class MainActivity extends Activity {
    private static final int PROJECTION_REQUEST = 31;
    private EditText hostInput, portInput, keyboardInput;
    private TextView status, targetStatus;
    private ImageView desktopImage;
    private LanWebSocket desktopSocket;
    private Bitmap desktopBitmap;
    private LinearLayout sessionPanel;
    private LinearLayout discoveredList;
    private TextView discoveryStatus;
    private int discoveryGeneration;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().setStatusBarColor(Color.rgb(10, 18, 36));
        getWindow().setNavigationBarColor(Color.rgb(10, 18, 36));
        buildUi();
        hostInput.setText(getPreferences(0).getString("host", ""));
        portInput.setText(getPreferences(0).getString("port", "9534"));
        refreshTargetStatus();
        scanLan(true);
    }

    private void buildUi() {
        int background = Color.rgb(10, 18, 36), card = Color.rgb(22, 34, 59), text = Color.rgb(235, 242, 255);
        ScrollView scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.setBackgroundColor(background);
        LinearLayout root = new LinearLayout(this); root.setOrientation(LinearLayout.VERTICAL); root.setPadding(dp(20), dp(28), dp(20), dp(28));
        scroll.addView(root); setContentView(scroll);
        if (Build.VERSION.SDK_INT >= 30) {
            scroll.setOnApplyWindowInsetsListener((view, insets) -> {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.statusBars() | WindowInsets.Type.navigationBars());
                root.setPadding(dp(20), bars.top + dp(18), dp(20), bars.bottom + dp(28));
                return insets;
            });
            scroll.requestApplyInsets();
        } else {
            scroll.setFitsSystemWindows(true);
        }
        TextView eyebrow = label("RDESK  ·  安全远程访问", 13, Color.rgb(104, 180, 255)); root.addView(eyebrow);
        TextView title = label("连接与共享", 30, text); title.setPadding(0, dp(7), 0, dp(8)); root.addView(title);
        TextView subtitle = label("让手机控制电脑，也可经您授权后让电脑控制手机。", 15, Color.rgb(159, 178, 209)); root.addView(subtitle);

        LinearLayout connection = card(card); root.addView(connection, spaced());
        connection.addView(label("电脑网关", 20, text));
        connection.addView(label("电脑与手机连接同一受信局域网。", 13, Color.rgb(159, 178, 209)));
        Button scan = button("扫描局域网电脑", false); connection.addView(scan, spaced()); scan.setOnClickListener(v -> scanLan(true));
        discoveryStatus = label("正在搜索同一局域网的电脑…", 13, Color.rgb(159, 178, 209)); connection.addView(discoveryStatus, spaced());
        discoveredList = new LinearLayout(this); discoveredList.setOrientation(LinearLayout.VERTICAL); connection.addView(discoveredList);
        hostInput = field("电脑 IPv4 地址，例如 192.168.1.10", false); connection.addView(hostInput, spaced());
        portInput = field("端口", false); portInput.setInputType(android.text.InputType.TYPE_CLASS_NUMBER); connection.addView(portInput, spaced());
        status = label("未连接", 14, Color.rgb(159, 178, 209)); connection.addView(status, spaced());
        Button connect = button("连接电脑屏幕", true); connection.addView(connect, spaced()); connect.setOnClickListener(v -> connectDesktop());

        sessionPanel = card(card); sessionPanel.setVisibility(View.GONE); root.addView(sessionPanel, spaced());
        sessionPanel.addView(label("电脑实时画面", 18, text));
        desktopImage = new ImageView(this); desktopImage.setAdjustViewBounds(true); desktopImage.setScaleType(ImageView.ScaleType.FIT_CENTER);
        desktopImage.setBackgroundColor(Color.BLACK); sessionPanel.addView(desktopImage, new LinearLayout.LayoutParams(-1, dp(360)));
        desktopImage.setOnTouchListener(this::handleDesktopTouch);
        LinearLayout actions = new LinearLayout(this); actions.setOrientation(LinearLayout.HORIZONTAL); sessionPanel.addView(actions, spaced());
        Button scrollUp = button("上滚", false), scrollDown = button("下滚", false), disconnect = button("断开", false);
        actions.addView(scrollUp, weighted()); actions.addView(scrollDown, weighted()); actions.addView(disconnect, weighted());
        scrollUp.setOnClickListener(v -> sendDesktop("wheel", null, null, null, 120));
        scrollDown.setOnClickListener(v -> sendDesktop("wheel", null, null, null, -120));
        disconnect.setOnClickListener(v -> disconnectDesktop());
        keyboardInput = field("输入文字", false); sessionPanel.addView(keyboardInput, spaced());
        LinearLayout keyboardActions = new LinearLayout(this); sessionPanel.addView(keyboardActions, spaced());
        Button type = button("发送文字", false), enter = button("回车", false), backspace = button("退格", false);
        keyboardActions.addView(type, weighted()); keyboardActions.addView(enter, weighted()); keyboardActions.addView(backspace, weighted());
        type.setOnClickListener(v -> { sendDesktopText(keyboardInput.getText().toString()); keyboardInput.setText(""); });
        enter.setOnClickListener(v -> sendVirtualKey(13)); backspace.setOnClickListener(v -> sendVirtualKey(8));

        LinearLayout target = card(card); root.addView(target, spaced());
        target.addView(label("共享这台手机", 20, text));
        target.addView(label("每次共享都会弹出 Android 屏幕采集授权。辅助功能需在系统设置中手动开启。", 13, Color.rgb(159, 178, 209)));
        targetStatus = label("", 14, Color.rgb(158, 220, 178)); target.addView(targetStatus, spaced());
        Button accessibility = button("开启辅助功能设置", false); target.addView(accessibility, spaced());
        accessibility.setOnClickListener(v -> startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)));
        Button share = button("批准并共享手机屏幕", true); target.addView(share, spaced());
        share.setOnClickListener(v -> requestProjection());
        Button stop = button("停止共享与电脑控制", false); target.addView(stop, spaced());
        stop.setOnClickListener(v -> { stopService(new Intent(this, ProjectionService.class)); refreshTargetStatus(); });
        root.addView(label("局域网连接当前使用明文传输，请只在受信网络使用。共享时可从通知栏随时停止。", 12, Color.rgb(145, 162, 190)), spaced());
    }

    private void connectDesktop() {
        try {
            String host = hostInput.getText().toString().trim();
            int port = Integer.parseInt(portInput.getText().toString().trim());
            String url = Protocol.desktopUrl(host, port);
            rememberGateway(host, port);
            disconnectDesktop(); status.setText("正在连接电脑…");
            desktopSocket = new LanWebSocket(url, new LanWebSocket.Listener() {
                @Override public void onText(String text) {
                    runOnUiThread(() -> {
                        if (text.contains("\"type\":\"ready\"")) { status.setText("已连接，正在等待电脑画面"); sessionPanel.setVisibility(View.VISIBLE); }
                        else if (text.contains("\"type\":\"error\"")) status.setText("网关拒绝连接");
                    });
                }
                @Override public void onBinary(byte[] bytes) {
                    Bitmap frame = BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
                    if (frame == null) return;
                    runOnUiThread(() -> { if (desktopBitmap != null && desktopBitmap != frame) desktopBitmap.recycle(); desktopBitmap = frame; desktopImage.setImageBitmap(frame); status.setText("电脑屏幕正在共享"); });
                }
                @Override public void onClosed(String reason) {
                    runOnUiThread(() -> { status.setText(reason); sessionPanel.setVisibility(View.GONE); });
                }
            });
            desktopSocket.connect();
        } catch (Exception error) { status.setText(error.getMessage()); }
    }

    private void scanLan(boolean extended) {
        if (discoveryStatus == null) return;
        int generation = ++discoveryGeneration;
        discoveryStatus.setText(extended ? "正在搜索附近的可路由网段…" : "正在搜索同一局域网的电脑…");
        discoveredList.removeAllViews();
        LanDiscovery.scan(hostInput.getText().toString().trim(), extended, new LanDiscovery.Listener() {
            private int found;
            @Override public void onGateway(String address, int port, String name) {
                runOnUiThread(() -> {
                    if (isDestroyed() || generation != discoveryGeneration) return;
                    found++;
                    discoveryStatus.setText("发现 " + found + " 台电脑，点选后直接连接");
                    Button device = button(name + "  ·  " + address + ":" + port, false);
                    discoveredList.addView(device, spaced());
                    device.setOnClickListener(v -> {
                        hostInput.setText(address);
                        portInput.setText(String.valueOf(port));
                        connectDesktop();
                    });
                });
            }
            @Override public void onFinished() {
                runOnUiThread(() -> { if (!isDestroyed() && generation == discoveryGeneration && found == 0) discoveryStatus.setText("未发现电脑，可手动输入地址或检查网关与防火墙"); });
            }
        });
    }

    private boolean handleDesktopTouch(View view, MotionEvent event) {
        if (desktopBitmap == null || desktopSocket == null) return false;
        if (event.getActionMasked() == MotionEvent.ACTION_DOWN) view.getParent().requestDisallowInterceptTouchEvent(true);
        if (event.getActionMasked() == MotionEvent.ACTION_UP || event.getActionMasked() == MotionEvent.ACTION_CANCEL) view.getParent().requestDisallowInterceptTouchEvent(false);
        RectF rect = imageRect(view.getWidth(), view.getHeight(), desktopBitmap.getWidth(), desktopBitmap.getHeight());
        float x = Protocol.normalized(event.getX() - rect.left, rect.width());
        float y = Protocol.normalized(event.getY() - rect.top, rect.height());
        String action = switch (event.getActionMasked()) {
            case MotionEvent.ACTION_DOWN -> "down";
            case MotionEvent.ACTION_MOVE -> "move";
            case MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> "up";
            default -> null;
        };
        if (action != null) sendDesktop("pointer", x, y, action, null);
        return true;
    }

    private RectF imageRect(int viewWidth, int viewHeight, int imageWidth, int imageHeight) {
        float scale = Math.min(viewWidth / (float) imageWidth, viewHeight / (float) imageHeight);
        float width = imageWidth * scale, height = imageHeight * scale;
        return new RectF((viewWidth - width) / 2, (viewHeight - height) / 2, (viewWidth + width) / 2, (viewHeight + height) / 2);
    }

    private void sendDesktop(String type, Float x, Float y, String action, Integer delta) {
        try {
            JSONObject json = new JSONObject().put("type", type);
            if (x != null) json.put("x", x);
            if (y != null) json.put("y", y);
            if (action != null) json.put("action", action);
            if (delta != null) json.put("delta", delta);
            if (desktopSocket != null) desktopSocket.sendText(json.toString());
        } catch (Exception ignored) {}
    }

    private void sendDesktopText(String text) {
        if (text.isEmpty() || text.getBytes(java.nio.charset.StandardCharsets.UTF_8).length > 512 || desktopSocket == null) return;
        try { desktopSocket.sendText(new JSONObject().put("type", "text").put("value", text).toString()); }
        catch (Exception ignored) {}
    }

    private void sendVirtualKey(int code) {
        try {
            if (desktopSocket == null) return;
            desktopSocket.sendText(new JSONObject().put("type", "key").put("code", code).put("pressed", true).toString());
            desktopSocket.sendText(new JSONObject().put("type", "key").put("code", code).put("pressed", false).toString());
        } catch (Exception ignored) {}
    }

    private void requestProjection() {
        try {
            String host = hostInput.getText().toString().trim();
            int port = Integer.parseInt(portInput.getText().toString().trim());
            Protocol.phonePublishUrl(host, port);
            rememberGateway(host, port);
            MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
            startActivityForResult(manager.createScreenCaptureIntent(), PROJECTION_REQUEST);
        } catch (Exception error) { Toast.makeText(this, error.getMessage(), Toast.LENGTH_LONG).show(); }
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PROJECTION_REQUEST) return;
        if (result != RESULT_OK || data == null) { targetStatus.setText("您未批准屏幕共享"); return; }
        Intent service = new Intent(this, ProjectionService.class)
                .putExtra(ProjectionService.EXTRA_RESULT, result).putExtra(ProjectionService.EXTRA_DATA, data)
                .putExtra(ProjectionService.EXTRA_HOST, hostInput.getText().toString().trim())
                .putExtra(ProjectionService.EXTRA_PORT, Integer.parseInt(portInput.getText().toString().trim()));
        startForegroundService(service);
        targetStatus.setText("正在共享手机屏幕");
    }

    private void rememberGateway(String host, int port) {
        getPreferences(0).edit().putString("host", host).putString("port", String.valueOf(port)).apply();
    }

    private void refreshTargetStatus() {
        if (targetStatus != null) targetStatus.setText((ProjectionService.isActive() ? "屏幕共享中" : "屏幕未共享") + " · " +
                (RemoteAccessibilityService.isConnected() ? "辅助功能已启用" : "辅助功能未启用"));
    }

    private void disconnectDesktop() {
        if (desktopSocket != null) { desktopSocket.close(); desktopSocket = null; }
        sessionPanel.setVisibility(View.GONE);
    }

    @Override protected void onResume() { super.onResume(); refreshTargetStatus(); }
    @Override protected void onDestroy() { disconnectDesktop(); if (desktopBitmap != null) desktopBitmap.recycle(); super.onDestroy(); }

    private LinearLayout card(int color) {
        LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.VERTICAL); layout.setPadding(dp(17), dp(18), dp(17), dp(18));
        android.graphics.drawable.GradientDrawable background = new android.graphics.drawable.GradientDrawable();
        background.setColor(color); background.setCornerRadius(dp(18)); layout.setBackground(background); return layout;
    }
    private TextView label(String content, int size, int color) { TextView view = new TextView(this); view.setText(content); view.setTextSize(size); view.setTextColor(color); return view; }
    private EditText field(String hint, boolean password) {
        EditText input = new EditText(this); input.setSingleLine(true); input.setTextColor(Color.WHITE); input.setHintTextColor(Color.rgb(138, 155, 182)); input.setHint(hint); input.setTextSize(15);
        if (password) input.setInputType(android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD);
        return input;
    }
    private Button button(String content, boolean primary) { Button button = new Button(this); button.setText(content); button.setAllCaps(false); button.setTextColor(Color.WHITE); button.setBackgroundTintList(android.content.res.ColorStateList.valueOf(primary ? Color.rgb(49, 101, 234) : Color.rgb(52, 73, 109))); return button; }
    private LinearLayout.LayoutParams spaced() { LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.topMargin = dp(14); return params; }
    private LinearLayout.LayoutParams weighted() { LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(0, -2, 1); params.rightMargin = dp(5); return params; }
    private int dp(float value) { return Math.round(getResources().getDisplayMetrics().density * value); }
}
