export default function Overlay() {
  return (
    <div className="overlay-root">
      <header className="overlay-header" data-tauri-drag-region>
        <span className="overlay-brand" data-tauri-drag-region>
          hint
        </span>
        <span className="overlay-status" data-tauri-drag-region>
          <i className="dot" /> 浮层已就绪
        </span>
      </header>

      <div className="overlay-transcript">
        <div className="line interviewer">
          <span className="who">面试官</span>
          <span className="what">语音链路将在 M3 接入，这里会显示实时转录。</span>
        </div>
        <div className="line me">
          <span className="who">我</span>
          <span className="what">麦克风与系统声双通道，各自独立识别。</span>
        </div>
      </div>

      <div className="overlay-answer">
        <p>答案会流式出现在这里。</p>
        <p className="placeholder">
          M0 只验证窗口、热键与配置；M1 接三协议流式回答，M3 接本地 SenseVoice。
        </p>
      </div>

      <footer className="overlay-footer">
        <span>Ctrl+Shift+Space 隐藏</span>
        <span>Esc</span>
      </footer>
    </div>
  );
}
