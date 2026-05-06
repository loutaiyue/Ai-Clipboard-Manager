import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useState,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

const LS_BASE = "clipAiApiBase";
const LS_KEY = "clipAiApiKey";
const LS_MODEL = "clipAiModel";
const LS_PROVIDER = "clipAiProvider";

const PROVIDERS = {
  openai: {
    label: "OpenAI / 兼容接口（DeepSeek、Qwen、Kimi、智谱等）",
    defaultBase: "https://api.openai.com/v1",
    defaultModel: "gpt-4o-mini",
    keyPlaceholder: "sk-...",
  },
  anthropic: {
    label: "Anthropic Claude",
    defaultBase: "https://api.anthropic.com/v1",
    defaultModel: "claude-3-5-sonnet-latest",
    keyPlaceholder: "sk-ant-...",
  },
  gemini: {
    label: "Google Gemini",
    defaultBase: "https://generativelanguage.googleapis.com/v1beta",
    defaultModel: "gemini-2.0-flash",
    keyPlaceholder: "AIza...",
  },
};

const TASK_LABEL = {
  translate_polish: "翻译并润色",
  code_diagnose: "代码诊断",
  extract_key_elements: "提取关键要素",
};

const clipInitial = { items: [], selectedId: null };

function clipReducer(state, action) {
  switch (action.type) {
    case "hydrate": {
      const { items } = action;
      if (!Array.isArray(items) || items.length === 0) return state;
      return { items, selectedId: items[0].id };
    }
    case "prependClip": {
      const { entry } = action;
      const next = [entry, ...state.items].slice(0, 200);
      return { items: next, selectedId: entry.id };
    }
    case "select": {
      if (state.selectedId === action.id) return state;
      return { ...state, selectedId: action.id };
    }
    default:
      return state;
  }
}

function formatTime(ms) {
  try {
    return new Intl.DateTimeFormat(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).format(new Date(ms));
  } catch {
    return "";
  }
}

function previewOneLine(text, max = 72) {
  const flat = text.replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  return `${flat.slice(0, max)}…`;
}

const HistoryRow = memo(function HistoryRow({ item, active, onSelect }) {
  return (
    <li>
      <button
        type="button"
        className={`history-item${active ? " is-active" : ""}`}
        onClick={() => onSelect(item.id)}
      >
        <span className="history-time">{formatTime(item.capturedAtMs)}</span>
        <span className="history-preview">{previewOneLine(item.text)}</span>
      </button>
    </li>
  );
});

export default function App() {
  const [{ items, selectedId }, dispatchClip] = useReducer(clipReducer, clipInitial);
  const [resultsById, setResultsById] = useState({});
  const [showSettings, setShowSettings] = useState(false);
  const [provider, setProvider] = useState("openai");
  const [apiBase, setApiBase] = useState("https://api.openai.com/v1");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("gpt-4o-mini");
  const [processing, setProcessing] = useState(false);
  const [actionError, setActionError] = useState("");

  useEffect(() => {
    try {
      const savedProvider = localStorage.getItem(LS_PROVIDER);
      const p = PROVIDERS[savedProvider] ? savedProvider : "openai";
      setProvider(p);
      setApiBase(localStorage.getItem(LS_BASE) || "https://api.openai.com/v1");
      setApiKey(localStorage.getItem(LS_KEY) || "");
      setModel(localStorage.getItem(LS_MODEL) || "gpt-4o-mini");
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(LS_PROVIDER, provider);
      localStorage.setItem(LS_BASE, apiBase);
      localStorage.setItem(LS_KEY, apiKey);
      localStorage.setItem(LS_MODEL, model);
    } catch {
      /* ignore */
    }
  }, [provider, apiBase, apiKey, model]);

  const applyProviderDefaults = useCallback((nextProvider) => {
    const cfg = PROVIDERS[nextProvider] || PROVIDERS.openai;
    setProvider(nextProvider);
    setApiBase(cfg.defaultBase);
    setModel(cfg.defaultModel);
  }, []);

  useEffect(() => {
    let active = true;
    let unlistenFn;

    const boot = async () => {
      try {
        const list = await invoke("get_clipboard_history");
        if (!active) return;
        if (Array.isArray(list) && list.length > 0) {
          dispatchClip({ type: "hydrate", items: list });
        }
      } catch {
        /* ignore */
      }
      if (!active) return;
      const unlisten = await listen("clipboard-text", (event) => {
        dispatchClip({ type: "prependClip", entry: event.payload });
      });
      if (!active) {
        unlisten();
        return;
      }
      unlistenFn = unlisten;
    };

    boot();
    return () => {
      active = false;
      unlistenFn?.();
    };
  }, []);

  const selected = useMemo(
    () => items.find((i) => i.id === selectedId) ?? items[0] ?? null,
    [items, selectedId],
  );

  const resultForSelected = selected ? resultsById[selected.id] : undefined;

  useEffect(() => {
    setActionError("");
  }, [selectedId]);

  const onSelect = useCallback((id) => {
    dispatchClip({ type: "select", id });
  }, []);

  const runAiTask = useCallback(async (task) => {
    if (!selected?.text) return;
    setActionError("");
    setProcessing(true);
    try {
      const out = await invoke("ai_summarize", {
        text: selected.text,
        task,
        provider,
        apiBase: apiBase.trim() || PROVIDERS[provider].defaultBase,
        apiKey,
        model: model.trim() || PROVIDERS[provider].defaultModel,
      });
      setResultsById((prev) => ({
        ...prev,
        [selected.id]: {
          task,
          output: out,
        },
      }));
    } catch (e) {
      const msg =
        (typeof e === "string" && e) ||
        e?.message ||
        (typeof e?.toString === "function" && e.toString() !== "[object Object]"
          ? e.toString()
          : null) ||
        "调用失败，请检查 API 与网络。";
      setActionError(msg);
    } finally {
      setProcessing(false);
    }
  }, [selected, provider, apiBase, apiKey, model]);

  return (
    <div className="app-root">
      <div className="shell-glow" aria-hidden />
      <div className="shell">
        <aside className="sidebar" aria-label="剪贴板历史">
          <header className="sidebar-header">
            <h1 className="app-title">剪贴板</h1>
            <p className="app-subtitle">从任意应用复制文本后会自动出现在此列表（已本地持久化）</p>
          </header>
          <ul className="history-list">
            {items.length === 0 ? (
              <li className="history-empty">
                切换到浏览器或其他软件，复制一段文字，条目会出现在这里。
              </li>
            ) : (
              items.map((item) => (
                <HistoryRow
                  key={item.id}
                  item={item}
                  active={selected?.id === item.id}
                  onSelect={onSelect}
                />
              ))
            )}
          </ul>
        </aside>
        <section className="preview-panel" aria-label="AI 处理预览">
          <header className="preview-header">
            <div className="preview-header-row">
              <div>
                <h2 className="preview-title">AI 多功能区</h2>
                <p className="preview-hint">
                  左侧为原始剪贴板内容；右侧可执行不同 AI 动作，不再仅限摘要。
                </p>
              </div>
              <div className="preview-actions">
                <button
                  type="button"
                  className="btn btn-ghost"
                  onClick={() => setShowSettings((v) => !v)}
                  aria-expanded={showSettings}
                >
                  {showSettings ? "收起 API 设置" : "API 设置"}
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={!selected?.text || processing}
                  onClick={() => runAiTask("translate_polish")}
                >
                  {processing ? "处理中…" : "翻译并润色"}
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={!selected?.text || processing}
                  onClick={() => runAiTask("code_diagnose")}
                >
                  {processing ? "处理中…" : "代码诊断"}
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={!selected?.text || processing}
                  onClick={() => runAiTask("extract_key_elements")}
                >
                  {processing ? "处理中…" : "提取关键要素"}
                </button>
              </div>
            </div>
            {showSettings ? (
              <div className="settings-panel">
                <label className="field">
                  <span className="field-label">提供商</span>
                  <select
                    className="field-input"
                    value={provider}
                    onChange={(e) => applyProviderDefaults(e.target.value)}
                  >
                    <option value="openai">{PROVIDERS.openai.label}</option>
                    <option value="anthropic">{PROVIDERS.anthropic.label}</option>
                    <option value="gemini">{PROVIDERS.gemini.label}</option>
                  </select>
                </label>
                <label className="field">
                  <span className="field-label">API Base（含 /v1，不含 /chat）</span>
                  <input
                    className="field-input"
                    value={apiBase}
                    onChange={(e) => setApiBase(e.target.value)}
                    placeholder={PROVIDERS[provider].defaultBase}
                    autoComplete="off"
                  />
                </label>
                <label className="field">
                  <span className="field-label">API Key</span>
                  <input
                    className="field-input"
                    type="password"
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    placeholder={PROVIDERS[provider].keyPlaceholder}
                    autoComplete="off"
                  />
                </label>
                <label className="field">
                  <span className="field-label">模型名</span>
                  <input
                    className="field-input"
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    placeholder={PROVIDERS[provider].defaultModel}
                    autoComplete="off"
                  />
                </label>
              </div>
            ) : null}
          </header>
          <div className="preview-body">
            {selected ? (
              <>
                <div className="preview-meta">
                  <span className="meta-label">捕获时间</span>
                  <span className="meta-value">{formatTime(selected.capturedAtMs)}</span>
                  <span className="meta-label">字符数</span>
                  <span className="meta-value">{selected.text.length}</span>
                </div>
                <div className="preview-section-label">原文</div>
                <pre className="preview-text">{selected.text}</pre>
                {resultForSelected ? (
                  <>
                    <div className="preview-section-label">
                      结果 · {TASK_LABEL[resultForSelected.task] || "AI 输出"}
                    </div>
                    <pre className="preview-text preview-text--summary">{resultForSelected.output}</pre>
                  </>
                ) : null}
                {actionError ? (
                  <p className="preview-error" role="alert">
                    {actionError}
                  </p>
                ) : null}
              </>
            ) : (
              <p className="preview-placeholder">暂无内容。请在系统中复制一段文本。</p>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
