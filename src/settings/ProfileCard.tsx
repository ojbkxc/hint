import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { describeError, isTauri, safeInvoke } from "../lib/tauri";
import type { Settings } from "./types";
import { deriveStatusTone, statusToneClass } from "./statusTone";

/** 与后端提示词截断保持一致：简历与项目资料各最多 6000 字。 */
const IMPORT_LIMIT = 6000;

type Props = {
  form: Settings;
  patch: (next: Partial<Settings>) => void;
};

type ImportField = "resume" | "projects";

export default function ProfileCard({ form, patch }: Props) {
  const [importing, setImporting] = useState<ImportField | "">("");
  const [status, setStatus] = useState("");
  const [organizing, setOrganizing] = useState(false);
  const [organizeStatus, setOrganizeStatus] = useState("");
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editingText, setEditingText] = useState("");

  /**
   * 改一条事实的正文。
   *
   * `patch` 是**浅合并**，所以必须重建整个 profile 对象——直接改 `form.profile.facts`
   * 里的元素等于原地改传入的 props，React 不会重渲染，保存时也未必带上。
   */
  function updateFact(index: number, next: Partial<{ text: string; tags: string[] }>) {
    if (!form.profile) return;
    const text = next.text ?? form.profile.facts[index]?.text ?? "";
    if (!text.trim()) return; // 空正文没有意义，也不该进 prompt
    const facts = form.profile.facts.map((fact, i) =>
      i === index ? { ...fact, ...next, text: text.trim() } : fact,
    );
    patch({ profile: { ...form.profile, facts } });
  }

  /** 删一条事实。删光则整个档案置空，退回「整段发送」路径。 */
  function removeFact(index: number) {
    if (!form.profile) return;
    const facts = form.profile.facts.filter((_, i) => i !== index);
    setEditingIndex(null);
    if (facts.length === 0) {
      patch({ profile: null });
      setOrganizeStatus("已删除全部事实，档案清空；之后按「简历 / 项目资料」整段发送。");
      return;
    }
    patch({ profile: { ...form.profile, facts } });
  }

  /**
   * 选多份文档 → 交给后端整理成事实列表。
   *
   * 把 `form.form`（当前表单，可能还没保存）一起传过去：用户常常是「刚粘完 Key
   * 就想整理」，此时配置还没落盘，用已保存的旧配置会连不上接口。后端 `organize_profile`
   * 与 `test_llm` 同一套「表单优先」处理。
   */
  async function organize() {
    if (!isTauri()) {
      setOrganizeStatus("纯浏览器预览无法打开文件对话框；安装版可直接整理。");
      return;
    }
    setOrganizing(true);
    setOrganizeStatus("");
    try {
      const selected = await open({
        multiple: true,
        directory: false,
        title: "选择要整理的资料（可多选）",
        filters: [
          {
            name: "资料文件（TXT / MD / DOCX / PDF）",
            extensions: ["txt", "md", "markdown", "docx", "pdf"],
          },
        ],
      });
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
      if (paths.length === 0) return;

      setOrganizeStatus(`正在整理 ${paths.length} 份文档…（模型调用，可能需要几十秒）`);
      // 只传文档，配置与 Key 用**已保存**的：整理是一次性离线动作，不追求
      // 「表单里刚粘的 Key 立刻可用」——那需要额外把 apiKey 透到这一层。
      // Key 没保存时后端会明确提示「请先保存」而不是静默失败。
      const result = await safeInvoke<Settings["profile"]>("organize_profile", {
        documents: paths,
      });
      if (!result.ok) {
        setOrganizeStatus(`整理失败：${result.error}`);
        return;
      }
      const profile = result.data;
      if (!profile || !Array.isArray(profile.facts) || profile.facts.length === 0) {
        setOrganizeStatus("整理失败：模型没有返回可解析的事实。");
        return;
      }
      patch({ profile });
      setOrganizeStatus(
        `已整理出 ${profile.facts.length} 条事实。记得点底部「保存全部配置」写入磁盘。`,
      );
    } catch (error) {
      setOrganizeStatus(`整理失败：${describeError(error)}`);
    } finally {
      setOrganizing(false);
    }
  }

  async function importFile(field: ImportField) {
    if (!isTauri()) {
      setStatus("纯浏览器预览无法打开文件对话框；安装版可直接导入。");
      return;
    }
    setImporting(field);
    setStatus("");
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: field === "resume" ? "选择简历文件" : "选择项目资料文件",
        filters: [
          {
            name: "资料文件（TXT / MD / DOCX / PDF）",
            extensions: ["txt", "md", "markdown", "docx", "pdf"],
          },
        ],
      });
      if (typeof selected !== "string") return;
      const result = await safeInvoke<string>("import_document_text", {
        path: selected,
      });
      if (!result.ok) {
        setStatus(`导入失败：${result.error}`);
        return;
      }
      const fullText = result.data.trim();
      const clipped =
        fullText.length > IMPORT_LIMIT
          ? `${fullText.slice(0, IMPORT_LIMIT)}\n（导入内容超过 ${IMPORT_LIMIT} 字，已截断）`
          : fullText;
      patch({ [field]: clipped } as Partial<Settings>);
      const name = selected.split(/[\\/]/).pop() ?? selected;
      setStatus(
        fullText.length > IMPORT_LIMIT
          ? `已导入 ${name}：原文 ${fullText.length} 字，已截取前 ${IMPORT_LIMIT} 字`
          : `已导入 ${name}（${fullText.length} 字）`,
      );
    } catch (error) {
      setStatus(`导入失败：${describeError(error)}`);
    } finally {
      setImporting("");
    }
  }

  return (
    <section className="settings-block">
      <div className="field-group">
        <p className="field-group-title">简历</p>
        <div className="field">
          <div className="field-head">
            <span className="field-label">面试官会围绕这里提问</span>
            <button
              type="button"
              className="ghost mini-import"
              onClick={() => void importFile("resume")}
              disabled={importing !== ""}
            >
              {importing === "resume" ? "导入中…" : "从文件导入"}
            </button>
          </div>
          <textarea
            id="profile-resume"
            className="form-input form-textarea"
            rows={8}
            value={form.resume}
            onChange={(event) => patch({ resume: event.target.value })}
            aria-label="简历"
            placeholder="粘贴简历文本，或点右上角「从文件导入」选择 TXT / MD / DOCX / PDF。简历与项目资料各最多 6000 字，超出部分会截断，并在给模型的提示词里标注「内容过长已截断」。"
          />
          <p
            className={`char-count${form.resume.length > IMPORT_LIMIT ? " is-over" : ""}`}
            aria-live="polite"
          >
            {form.resume.length} / {IMPORT_LIMIT}
          </p>
        </div>
      </div>

      <div className="field-group">
        <p className="field-group-title">项目资料</p>
        <div className="field">
          <div className="field-head">
            <span className="field-label">背景、技术栈、你负责的部分与量化结果</span>
            <button
              type="button"
              className="ghost mini-import"
              onClick={() => void importFile("projects")}
              disabled={importing !== ""}
            >
              {importing === "projects" ? "导入中…" : "从文件导入"}
            </button>
          </div>
          <textarea
            id="profile-projects"
            className="form-input form-textarea"
            rows={7}
            value={form.projects}
            onChange={(event) => patch({ projects: event.target.value })}
            aria-label="项目资料"
            placeholder="项目背景、技术栈、你负责的部分、难点与量化结果。也可从 TXT / MD / DOCX / PDF 导入；最多 6000 字，超出会截断并标注。"
          />
          <p
            className={`char-count${form.projects.length > IMPORT_LIMIT ? " is-over" : ""}`}
            aria-live="polite"
          >
            {form.projects.length} / {IMPORT_LIMIT}
          </p>
        </div>

        {status && <p className={statusToneClass(deriveStatusTone(status))}>{status}</p>}
        <p className="muted small">
          导入的文件只在本机解析（PDF / Word 文本提取全部离线完成），资料不会上传到
          除你所选模型接口以外的任何地方。
        </p>
      </div>

      {/* 整理：把多份文档压成可检索的紧凑档案。
          为什么值得单独一步——见 docs：原来简历+项目各 6000 字**每次请求全量发送**，
          中文 12000 字约 8–12k tokens，首字延迟与 prompt 长度近似成正比。整理之后
          每轮只发常驻段 + 命中片段（约 2600 字）。 */}
      <div className="field-group">
        <p className="field-group-title">面试档案（整理，可选）</p>
        <p className="muted small">
          选多份资料（简历、项目文档、复盘笔记…），让模型压成一份紧凑档案。之后提问时
          只按问题检索相关片段发送，首字更快；不整理则仍按上面「简历 / 项目资料」整段
          发送（较慢但不用额外操作）。
        </p>

        <div className="profile-organize-row">
          <button
            type="button"
            className="primary"
            onClick={() => void organize()}
            disabled={organizing}
          >
            {organizing ? "整理中…" : "选择文档并整理"}
          </button>
          {form.profile && (
            <button
              type="button"
              className="ghost"
              onClick={() => {
                patch({ profile: null });
                setOrganizeStatus("已清除档案；之后按「简历 / 项目资料」整段发送。");
              }}
              disabled={organizing}
            >
              清除档案
            </button>
          )}
        </div>

        {organizeStatus && (
          <p className={statusToneClass(deriveStatusTone(organizeStatus))}>{organizeStatus}</p>
        )}

        {form.profile && form.profile.facts.length > 0 && (
          <div className="profile-facts">
            <p className="muted small">
              共 {form.profile.facts.length} 条事实
              {form.profile.sources.length > 0 &&
                `，基于 ${form.profile.sources.join("、")}`}
              。提问时按关键词挑相关几条发送。
              <br />
              整理难免有偏差（写错数字、编出不存在的细节），**逐条可改可删**——留下
              不准确的条目，面试时会被直接念出去。
            </p>
            <ul className="profile-fact-list">
              {form.profile.facts.map((fact, index) => (
                <li key={index}>
                  <span className="profile-fact-kind">{fact.kind}</span>
                  {editingIndex === index ? (
                    <span className="profile-fact-edit">
                      <textarea
                        className="form-input profile-fact-input"
                        rows={2}
                        value={editingText}
                        onChange={(event) => setEditingText(event.target.value)}
                        autoFocus
                        aria-label={`编辑第 ${index + 1} 条事实`}
                      />
                      <button
                        type="button"
                        className="ghost mini-import"
                        onClick={() => {
                          updateFact(index, { text: editingText.trim() });
                          setEditingIndex(null);
                        }}
                        disabled={!editingText.trim()}
                      >
                        保存
                      </button>
                      <button
                        type="button"
                        className="ghost mini-import"
                        onClick={() => setEditingIndex(null)}
                      >
                        取消
                      </button>
                    </span>
                  ) : (
                    <span className="profile-fact-body">
                      <button
                        type="button"
                        className="profile-fact-text"
                        onClick={() => {
                          setEditingIndex(index);
                          setEditingText(fact.text);
                        }}
                        title="点击修改这条"
                      >
                        {fact.text}
                      </button>
                      <button
                        type="button"
                        className="profile-fact-remove"
                        onClick={() => removeFact(index)}
                        title="删除这条（不进 prompt）"
                        aria-label={`删除第 ${index + 1} 条事实`}
                      >
                        ×
                      </button>
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </section>
  );
}