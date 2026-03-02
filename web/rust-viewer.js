const wsUrl = `ws://${location.hostname || 'localhost'}:9527`;
const statusEl = document.getElementById('status');
const deviceEl = document.getElementById('device');
const frameEl = document.getElementById('frame');
const placeholderEl = document.getElementById('placeholder');
const metaEl = document.getElementById('meta');

const state = {
  ws: null,
  selectedId: '',
  devices: new Map()
};

function connect() {
  const ws = new WebSocket(wsUrl);
  state.ws = ws;

  ws.onopen = () => {
    statusEl.textContent = 'WS: connected';
  };

  ws.onclose = () => {
    statusEl.textContent = 'WS: disconnected, reconnecting...';
    setTimeout(connect, 2000);
  };

  ws.onerror = () => {
    statusEl.textContent = 'WS: error';
  };

  ws.onmessage = (evt) => {
    let msg;
    try {
      msg = JSON.parse(evt.data);
    } catch {
      return;
    }

    if (msg.type === 'system' && msg.action === 'connected') {
      send({
        type: 'device',
        action: 'register',
        payload: { type: 'controller', name: 'Rust Viewer' }
      });
      send({ type: 'device', action: 'getDeviceList', payload: {} });
      return;
    }

    if (msg.type === 'device' && (msg.action === 'registered' || msg.action === 'deviceList')) {
      const list = msg.payload?.deviceList || [];
      syncDevices(list);
      return;
    }

    if (msg.type === 'device' && msg.action === 'offline') {
      state.devices.delete(msg.payload?.deviceId);
      renderDevices();
      return;
    }

    if (msg.type === 'stream' && msg.action === 'frame') {
      const payload = msg.payload || {};
      if (!payload.deviceId || !payload.image) return;
      if (state.selectedId && payload.deviceId !== state.selectedId) return;
      frameEl.src = `data:image/jpeg;base64,${payload.image}`;
      frameEl.style.display = 'block';
      placeholderEl.style.display = 'none';
      metaEl.textContent = `${payload.deviceName || payload.deviceId} ${payload.width || ''}x${payload.height || ''}`;
    }
  };
}

function send(data) {
  if (state.ws?.readyState === WebSocket.OPEN) {
    state.ws.send(JSON.stringify(data));
  }
}

function syncDevices(list) {
  state.devices.clear();
  for (const d of list) {
    if (d.name?.includes('Rust Agent') || d.name?.includes('agent-rust')) {
      state.devices.set(d.id, d);
    }
  }
  renderDevices();
}

function renderDevices() {
  const items = Array.from(state.devices.values());
  deviceEl.innerHTML = items.map((d) => `<option value="${d.id}">${d.name}</option>`).join('');
  if (!state.selectedId || !state.devices.has(state.selectedId)) {
    state.selectedId = items[0]?.id || '';
  }
  deviceEl.value = state.selectedId;
}

deviceEl.addEventListener('change', () => {
  state.selectedId = deviceEl.value;
  frameEl.style.display = 'none';
  placeholderEl.style.display = 'block';
});

connect();
