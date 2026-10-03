import { FormEvent, useState } from "react";
import { api } from "../api";
import { BrandMark } from "./BrandMark";
import { AppIcon } from "./AppIcon";

interface LoginPageProps {
  onAuthenticated: () => void;
}

export function LoginPage({ onAuthenticated }: LoginPageProps) {
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!password || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const result = await api.login(password);
      if (result.authenticated) onAuthenticated();
      else setError("认证失败，请检查管理员密码");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "无法连接 PingLake Hub");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <main className="login-shell">
      <section className="login-panel" aria-labelledby="login-title">
        <div className="login-panel-inner">
          <BrandMark size="md" showWordmark className="login-brand" />
          <div className="login-heading">
            <p className="eyebrow">MONITORING CONSOLE</p>
            <h1 id="login-title">欢迎回来</h1>
            <p>使用 Hub 管理员密码进入监控控制台</p>
          </div>
          <form onSubmit={submit} className="login-form">
            <label htmlFor="password">管理员密码</label>
            <div className="password-field">
              <AppIcon name="lock" size={17} />
              <input
                id="password"
                type={showPassword ? "text" : "password"}
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoComplete="current-password"
                autoFocus
                placeholder="输入管理员密码"
                required
              />
              <button
                type="button"
                className="icon-button subtle"
                onClick={() => setShowPassword((visible) => !visible)}
                aria-label={showPassword ? "隐藏密码" : "显示密码"}
                title={showPassword ? "隐藏密码" : "显示密码"}
              >
                {showPassword ? <AppIcon name="eye-off" size={17} /> : <AppIcon name="eye" size={17} />}
              </button>
            </div>
            {error && <div className="form-error" role="alert">{error}</div>}
            <button className="primary-button login-button" type="submit" disabled={submitting || !password}>
              {submitting && <AppIcon name="loader" className="spin" size={17} />}
              <span>{submitting ? "正在验证" : "进入控制台"}</span>
              {!submitting && <AppIcon name="external" size={17} />}
            </button>
          </form>
          <p className="login-footnote"><AppIcon name="shield" size={14} />连接由 PingLake Hub 本地验证</p>
        </div>
      </section>
      <aside className="login-environment" aria-label="PingLake 监控能力">
        <div className="login-orbit login-orbit-one" aria-hidden="true" />
        <div className="login-orbit login-orbit-two" aria-hidden="true" />
        <div className="login-signal-card">
          <div className="login-signal-header"><span className="signal-live-dot" />LIVE MONITORING</div>
          <div className="pulse-trace" aria-hidden="true" />
          <div className="login-signal-footer"><span><AppIcon name="sparkle" size={14} />Operational clarity</span><strong>READY</strong></div>
        </div>
        <div className="login-environment-copy"><span>HUB / AGENT STATUS</span><strong>Everything in view.</strong><p>让基础设施的每一个信号，都在一个清晰的工作台里。</p></div>
      </aside>
    </main>
  );
}
