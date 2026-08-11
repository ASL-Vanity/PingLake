import { Check, Clipboard, Server, TriangleAlert } from "lucide-react";
import { useState } from "react";

function agentConfig(hubUrl: string, nodeName: string, allowInsecureHttp: boolean) {
  return [
    `hub_url = "${hubUrl}"`,
    'enrollment_token = "<管理员配置的注册令牌>"',
    `name = "${nodeName}"`,
    "interval_secs = 5",
    "insecure_skip_verify = false",
    `allow_insecure_http = ${allowInsecureHttp}`,
  ].join("\n");
}

function ConfigExample({ title, path, content }: { title: string; path: string; content: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(content);
      } else {
        const textarea = document.createElement("textarea");
        textarea.value = content;
        textarea.style.position = "fixed";
        textarea.style.opacity = "0";
        document.body.appendChild(textarea);
        textarea.select();
        const succeeded = document.execCommand("copy");
        textarea.remove();
        if (!succeeded) return;
      }
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_500);
    } catch {
      // Clipboard permissions vary by browser and deployment context.
    }
  };

  return (
    <div className="config-example">
      <div className="config-heading">
        <div>
          <strong>{title}</strong>
          <span>{path}</span>
        </div>
        <button className="icon-button" type="button" onClick={() => void copy()} title="复制配置" aria-label={`复制${title}配置`}>
          {copied ? <Check size={16} /> : <Clipboard size={16} />}
        </button>
      </div>
      <pre><code>{content}</code></pre>
      <p><code>PINGLAKE_CONFIG</code> 指向该文件后运行 <code>pinglake-agent</code></p>
    </div>
  );
}

export function EmptyNodes() {
  const hubUrl = window.location.origin;
  const usingHttp = window.location.protocol === "http:";
  const allowInsecureHttp = usingHttp && !isLoopbackOrigin(hubUrl);

  return (
    <section className="empty-state">
      <div className="empty-icon"><Server size={24} /></div>
      <h2>尚未接入节点</h2>
      <p>在 Windows 或 Linux 主机创建 Agent 配置，然后启动 <code>pinglake-agent</code>。</p>
      <div className="config-grid">
        <ConfigExample title="Linux" path="/etc/pinglake/agent.toml" content={agentConfig(hubUrl, "linux-node-01", allowInsecureHttp)} />
        <ConfigExample title="Windows" path="C:\\ProgramData\\PingLake\\agent.toml" content={agentConfig(hubUrl, "windows-node-01", allowInsecureHttp)} />
      </div>
      <div className="security-note">
        注册令牌由管理员在 Hub 端配置。不要使用示例占位符，也不要把令牌提交到代码仓库。
      </div>
      {usingHttp && (
        <div className="http-security-warning" role="alert">
          <TriangleAlert size={16} />
          <span>{allowInsecureHttp ? "当前 Hub 使用非回环 HTTP，示例会显式允许不安全 HTTP。" : "当前 Hub 使用 HTTP。"}注册令牌和指标传输不受 TLS 保护；仅限隔离测试网络，生产环境请使用 HTTPS 地址。</span>
        </div>
      )}
    </section>
  );
}

function isLoopbackOrigin(origin: string): boolean {
  const hostname = new URL(origin).hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return hostname === "localhost" || hostname === "::1" || hostname.startsWith("127.");
}
