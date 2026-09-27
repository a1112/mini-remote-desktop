package com.a1112.rdeskmobile;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.GestureDescription;
import android.graphics.Path;
import android.graphics.Rect;
import android.os.Bundle;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;
import org.json.JSONObject;

public final class RemoteAccessibilityService extends AccessibilityService {
    private static volatile RemoteAccessibilityService instance;
    private static volatile boolean remoteActive;

    static boolean isConnected() { return instance != null; }
    static void setRemoteActive(boolean active) { remoteActive = active; }

    static void acceptControl(String message) {
        RemoteAccessibilityService service = instance;
        if (!remoteActive || service == null) return;
        service.getMainExecutor().execute(() -> service.executeControl(message));
    }

    @Override public void onServiceConnected() { super.onServiceConnected(); instance = this; }
    @Override public void onAccessibilityEvent(AccessibilityEvent event) {}
    @Override public void onInterrupt() {}
    @Override public void onDestroy() { instance = null; remoteActive = false; super.onDestroy(); }

    private void executeControl(String message) {
        if (!remoteActive) return;
        try {
            JSONObject json = new JSONObject(message);
            String type = json.optString("type");
            if ("back".equals(type)) { performGlobalAction(GLOBAL_ACTION_BACK); return; }
            if ("home".equals(type)) { performGlobalAction(GLOBAL_ACTION_HOME); return; }
            if ("text".equals(type)) {
                String text = json.optString("text", "");
                if (text.length() > 512) return;
                AccessibilityNodeInfo root = getRootInActiveWindow();
                if (root == null) return;
                AccessibilityNodeInfo focus = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT);
                if (focus != null) {
                    Bundle arguments = new Bundle(); arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
                    focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments);
                    focus.recycle();
                }
                root.recycle();
                return;
            }
            if (!"tap".equals(type) && !"swipe".equals(type)) return;
            double x = json.optDouble("x", -1), y = json.optDouble("y", -1);
            if (!valid(x) || !valid(y)) return;
            Rect bounds = ((WindowManager) getSystemService(WINDOW_SERVICE)).getMaximumWindowMetrics().getBounds();
            float startX = (float) (x * (bounds.width() - 1)), startY = (float) (y * (bounds.height() - 1));
            Path path = new Path(); path.moveTo(startX, startY);
            long duration = 80;
            if ("swipe".equals(type)) {
                double endX = json.optDouble("endX", -1), endY = json.optDouble("endY", -1);
                if (!valid(endX) || !valid(endY)) return;
                path.lineTo((float) (endX * (bounds.width() - 1)), (float) (endY * (bounds.height() - 1)));
                duration = 350;
            }
            GestureDescription gesture = new GestureDescription.Builder()
                    .addStroke(new GestureDescription.StrokeDescription(path, 0, duration)).build();
            dispatchGesture(gesture, null, null);
        } catch (Exception ignored) {}
    }

    private static boolean valid(double value) { return Double.isFinite(value) && value >= 0 && value <= 1; }
}
