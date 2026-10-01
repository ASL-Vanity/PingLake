import { FormEvent, useState } from "react";
import { Eye, EyeOff, LoaderCircle, LockKeyhole } from "lucide-react";
import { api } from "../api";

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
        <div className="brand-lockup login-brand">
          <span className="brand-mark"><img src="/pinglake-mark.svg" alt="" /></span>
          <span>PingLake<small>监控控制台</small></span>
        </div>
        <div className="login-heading">
          <h1 id="login-title">登录控制台</h1>
          <p>使用 Hub 管理员密码继续</p>
        </div>
        <form onSubmit={submit} className="login-form">
          <label htmlFor="password">管理员密码</label>
          <div className="password-field">
            <LockKeyhole size={17} aria-hidden="true" />
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
              {showPassword ? <EyeOff size={17} /> : <Eye size={17} />}
            </button>
          </div>
          {error && <div className="form-error" role="alert">{error}</div>}
          <button className="primary-button login-button" type="submit" disabled={submitting || !password}>
            {submitting && <LoaderCircle className="spin" size={17} />}
            {submitting ? "正在验证" : "登录"}
          </button>
        </form>
      </section>
    </main>
  );
}
