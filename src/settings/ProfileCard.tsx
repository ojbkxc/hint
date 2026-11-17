import type { Settings } from "./types";

type Props = {
  form: Settings;
  patch: (next: Partial<Settings>) => void;
};

export default function ProfileCard({ form, patch }: Props) {
  return (
    <section className="card">
      <h2>资料注入</h2>
      <label>
        简历
        <textarea
          rows={9}
          value={form.resume}
          onChange={(event) => patch({ resume: event.target.value })}
          placeholder="粘贴简历文本。v1 阶段直接全文注入，超出 token 预算时会按「简历 → 项目 → 其他」优先级截断。"
        />
      </label>
      <label>
        项目资料
        <textarea
          rows={7}
          value={form.projects}
          onChange={(event) => patch({ projects: event.target.value })}
          placeholder="项目背景、技术栈、你负责的部分、难点与量化结果。"
        />
      </label>
      <p className="muted small">
        资料只存在本机，不会上传到除你所选模型接口以外的任何地方。
      </p>
    </section>
  );
}