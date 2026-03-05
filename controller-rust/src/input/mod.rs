/// 输入事件转发模块
///
/// 将本地键盘和鼠标事件转发到远程代理
use common_control_proto::ControlEvent;
use tokio::sync::mpsc;

/// 输入事件
#[derive(Debug, Clone)]
pub enum InputEvent {
    /// 键盘事件
    Keyboard { key: u32, pressed: bool },
    /// 鼠标移动
    MouseMove { x: i32, y: i32 },
    /// 鼠标按钮
    MouseButton { button: u32, pressed: bool },
    /// 鼠标滚轮
    MouseWheel { delta: i32 },
}

impl From<InputEvent> for ControlEvent {
    fn from(value: InputEvent) -> Self {
        match value {
            InputEvent::Keyboard { key, pressed } => Self::Key { key, pressed },
            InputEvent::MouseMove { x, y } => Self::MouseMove { x, y },
            InputEvent::MouseButton { button, pressed } => Self::MouseButton {
                button: button as u8,
                pressed,
            },
            InputEvent::MouseWheel { delta } => Self::MouseWheel { delta },
        }
    }
}

/// 输入管理器
pub struct InputManager {
    event_tx: mpsc::Sender<InputEvent>,
}

impl InputManager {
    /// 创建新的输入管理器
    pub fn new() -> (Self, mpsc::Receiver<InputEvent>) {
        let (event_tx, event_rx) = mpsc::channel(100);
        (Self { event_tx }, event_rx)
    }

    /// 发送键盘事件
    pub async fn send_key(&self, key: u32, pressed: bool) -> anyhow::Result<()> {
        self.event_tx
            .send(InputEvent::Keyboard { key, pressed })
            .await
            .map_err(|e| anyhow::anyhow!("failed to send key event: {}", e))
    }

    /// 发送鼠标移动事件
    pub async fn send_mouse_move(&self, x: i32, y: i32) -> anyhow::Result<()> {
        self.event_tx
            .send(InputEvent::MouseMove { x, y })
            .await
            .map_err(|e| anyhow::anyhow!("failed to send mouse move event: {}", e))
    }

    /// 发送鼠标按钮事件
    pub async fn send_mouse_button(&self, button: u32, pressed: bool) -> anyhow::Result<()> {
        self.event_tx
            .send(InputEvent::MouseButton { button, pressed })
            .await
            .map_err(|e| anyhow::anyhow!("failed to send mouse button event: {}", e))
    }

    /// 发送鼠标滚轮事件
    pub async fn send_mouse_wheel(&self, delta: i32) -> anyhow::Result<()> {
        self.event_tx
            .send(InputEvent::MouseWheel { delta })
            .await
            .map_err(|e| anyhow::anyhow!("failed to send mouse wheel event: {}", e))
    }
}

impl Default for InputManager {
    fn default() -> Self {
        let (s, _) = Self::new();
        s
    }
}
