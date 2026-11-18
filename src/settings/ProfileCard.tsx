import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { describeError, isTauri, safeInvoke } from "../lib/tauri";
import type { Settings } from "./types";

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
    <section className="card">
      <h2>资料注入</h2>
      <label>
        <span className="field-head">
          <span>简历</span>
          <button
            type="button"
            className="ghost mini-import"
            onClick={() => void importFile("resume")}
            disabled={importing !== ""}
          >
            {importing === "resume" ? "导入中…" : "从文件导入"}
          </button>
        </span>
        <textarea
          rows={9}
          value={form.resume}
          onChange={(event) => patch({ resume: event.target.value })}
          placeholder="粘贴简历文本，或点右上角「从文件导入」选择 TXT / MD / DOCX / PDF。简历与项目资料各最多 6000 字，超出部分会截断，并在给模型的提示词里标注「内容过长已截断」。"
        />
      </label>
      <label>
        <span className="field-head">
          <span>项目资料</span>
          <button
            type="button"
            className="ghost mini-import"
            onClick={() => void importFile("projects")}
            disabled={importing !== ""}
          >
            {importing === "projects" ? "导入中…" : "从文件导入"}
          </button>
        </span>
        <textarea
          rows={7}
          value={form.projects}
          onChange={(event) => patch({ projects: event.target.value })}
          placeholder="项目背景、技术栈、你负责的部分、难点与量化结果。也可从 TXT / MD / DOCX / PDF 导入；最多 6000 字，超出会截断并标注。"
        />
      </label>
      {status && <p className="status">{status}</p>}
      <p className="muted small">
        导入的文件只在本机解析（PDF / Word 文本提取全部离线完成），资料不会上传到
        除你所选模型接口以外的任何地方。
      </p>
    </section>
  );
}
