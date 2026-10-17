import React from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
const root = document.getElementById('root');
if (!root) throw new Error('Missing application root');
ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <main><h1>Open Meeting Memo</h1><p>桌面应用正在开发，尚未连接会议功能。</p><p>启动此工程不会启动服务、录音或模型。</p></main>
  </React.StrictMode>,
);
