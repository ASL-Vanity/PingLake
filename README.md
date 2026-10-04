# PingLake

PingLake 是面向少量到中等规模 Windows/Linux 主机的实时状态监控系统。

> **v0.3.0** · [GitHub Releases](https://github.com/ASL-Vanity/PingLake/releases) · [中文使用手册](docs/USER_GUIDE.md) · [升级说明](docs/v0.3.0-upgrade.md)

它由一个中心 Hub、每台主机上的只读 Agent 和同源 Web 控制台组成。

Agent 主动连接 Hub，因此被监控节点不需要域名、固定公网地址或入站端口。所有 Agent 连接同一个 Hub；跨公网部署时必须使用 HTTPS。

## v0.3.0 控制台与监控工作台

本版本保留认证、节点监控、实时推送、告警、分组、历史图表和通知配置能力，采用紧凑顶部导航、移动端抽屉与底部导航、卡片和列表节点视图、节点详情页签、响应式登录页和统一异常状态。主题通过可展开按钮切换 Obsidian 曜石、Porcelain 云瓷、Lagoon 深海、Amber 琥珀及跟随系统；主题保存在 `localStorage["pinglake.theme"]`，旧的 `light`、`dark`、`midnight`、`circuit` 值会自动迁移。BrandMark、favicon 和 Apple Touch Icon 使用同一套新标记。

完整的安装、节点接入、主题、SMTP、Webhook、告警阈值、升级、备份和排错步骤见 [详细使用手册](docs/USER_GUIDE.md)。

## v0.3.0 扩展监控能力

- 每核心 CPU、CPU user/system/IOWait、可用内存、Swap、磁盘 IO/IOPS/延迟/队列、inode、网卡错误/丢弃和 TCP 状态。
- Linux systemd 与 Windows SCM 服务检查、精确进程名检查、本机 TCP/UDP 监听端口检查。
- ICMP、TCP、HTTP(S) 和 DNS 主动探测；DNS 只解析，不连接返回地址，并保留 RCODE 与答案。
- 配置 revision、Agent 应用 revision、采集质量、上报成功率、队列长度、重试、丢弃和样本年龄。
- 服务、探测、进程和本机端口历史，以及 P50/P95/P99、成功率、覆盖率和未知时长。
- 浏览器到节点 HTTPS 测点延迟；它是浏览器 HTTP 往返时间，不是 ICMP 或 Agent→Hub 延迟。
- Monitoring schema v2 与 v1 Agent/Hub 兼容；旧报告继续可读，v2-only 字段在 v1 连接中会被安全投影掉。
- Agent 持久化最多 64 条待发送报告，支持重试、过期和溢出丢弃，Hub 仍是已接受数据的事实来源。

检查间隔为 10–86,400 秒，默认 30 秒；超时为 1–10,000 毫秒且必须短于间隔。每类检查每个节点最多 32 个，私网和 loopback 目标默认禁止。

## 功能

- Windows 与 Linux 统一 Agent
- CPU、内存、Swap、磁盘、网络、温度、负载、进程数和运行时间
- 实时状态推送与 1/6/24 小时历史图表
- CPU、内存、磁盘、温度和离线告警
- 通用 JSON Webhook 与 SMTP 邮件通知（活动告警和恢复事件）
- SQLite 单文件存储，默认保留 7 天原始数据
- Agent 密钥哈希存储、管理员会话、只读采集
- Docker Hub 部署与 Windows/Linux 系统服务安装脚本
- v0.3.0 全面重做的响应式控制台、四套主题、卡片/列表节点视图和新的 PingLake 品牌资源

## 快速启动 Hub

1. 复制 `.env.example` 为 `.env`，设置 `PINGLAKE_ADMIN_PASSWORD`、`PINGLAKE_ENROLLMENT_TOKEN` 和 `PINGLAKE_DOMAIN`。两个秘密值至少 16 个字节；PingLake 没有默认密码。
2. 将域名 A/AAAA 记录指向 Hub 云服务器，并开放 TCP `80/443` 与 UDP `443`。
3. 启动：

```powershell
docker compose -f docker-compose.yml -f deploy/docker-compose.tls.yml up -d --build
```

访问 `https://你的域名`，在登录页输入 `.env` 中的 `PINGLAKE_ADMIN_PASSWORD`。当前版本只有管理员密码登录，没有 username 字段、多用户账户或密码找回功能。

仅限可信内网或临时测试的 IP 模式：

```dotenv
PINGLAKE_PUBLISH_ADDR=0.0.0.0
PINGLAKE_COOKIE_SECURE=false
```

```powershell
docker compose up -d --build hub
```

然后访问 `http://Hub-IP:8090`。不要在公网长期使用明文 HTTP。

基础 Compose 文件不加载 Caddy，因此无域名的 IP 模式不会要求 `PINGLAKE_DOMAIN`。TLS 覆盖文件会在启动前拒绝空域名。Compose 的 `healthcheck` 会访问 Hub API 并读取 settings 表，能确认服务和数据库基本可读；它不能证明 SQLite 在磁盘满、只读或运行时 I/O 失败后仍可写入。生产环境仍应对 Hub 建立外部 HTTPS 探测并配置磁盘告警。

## 安装 Agent

先构建或取得对应系统的 `pinglake-agent` 二进制。Windows 上执行 `.\scripts\build-release.ps1` 会创建一个全新的 `release` 目录，其中包含 Windows 二进制、Linux x86_64 二进制及 `SHA256SUMS.txt`。该目录用于分发二进制和 Agent 安装器；Docker Hub 部署仍应在完整源代码目录中执行。所有节点都指向同一个 Hub URL；节点自身是否有域名不影响监控。

Linux：

```bash
sudo ./install-agent-linux.sh \
  --binary ./pinglake-agent \
  --hub-url https://monitor.example.com \
  --name vm-linux-01
```

脚本会以隐藏输入方式询问注册令牌，也可以通过仅 root 可读的 `--enrollment-token-file` 提供。Linux 发布产物为 systemd/x86_64；从 HTTPS URL 下载二进制时必须同时传入发布清单中的 `--sha256` 值：

```bash
sudo ./install-agent-linux.sh \
  --binary https://downloads.example.com/pinglake-agent-linux-amd64 \
  --sha256 "$(awk '$2 == "pinglake-agent-linux-amd64" {print $1}' SHA256SUMS.txt)" \
  --hub-url https://monitor.example.com
```

Windows 管理员 PowerShell：

```powershell
.\install-agent-windows.ps1 `
  -Binary .\pinglake-agent.exe `
  -HubUrl https://monitor.example.com `
  -Name vm-windows-01
```

脚本会弹出安全输入提示读取注册令牌，也可以通过 `-EnrollmentTokenFile` 指定受限权限文件。从 HTTPS URL 下载时必须传入 `-Sha256`，并使用同一发布包 `SHA256SUMS.txt` 中 `pinglake-agent.exe` 的值。

注册令牌保存在仅管理员和服务账户可读取的配置文件中，不会出现在服务进程参数。所有节点完成注册后，应更换 Hub 的 `PINGLAKE_ENROLLMENT_TOKEN` 并重启 Hub。

可信内网或临时测试的 IP 模式必须显式承认明文风险：Linux 追加 `--allow-insecure-http`，Windows 追加 `-AllowInsecureHttp`。此模式会通过 HTTP 传输注册令牌、Agent 凭据和指标，不能用于公网或不可信网络。

## 本地开发与验证

```powershell
cd web
npm install
npm run build
npm run typecheck
npm test
cd ..
$env:PINGLAKE_ADMIN_PASSWORD='development-password'
$env:PINGLAKE_ENROLLMENT_TOKEN='development-enrollment-token'
$env:PINGLAKE_ALLOW_WEAK_ADMIN_PASSWORD='true' # Only for isolated local tests.
$env:PINGLAKE_COOKIE_SECURE='false'
cargo run -p pinglake-hub
```

扩展监控的受控 E2E：

```powershell
.\scripts\e2e-smoke.ps1 -Port 18090 -LeaveRunning
.\scripts\e2e-monitoring.ps1 -RunRoot <run-root-from-smoke>
```

这些测试使用项目本地 loopback 目标，不代表生产 HTTPS 浏览器测点、Linux 权限、Windows LocalService 或真实外部节点已经验收。请查看 [monitoring-validation.md](docs/monitoring-validation.md) 和 [production-verification.md](docs/production-verification.md) 区分自动化证据与待完成环境检查。

另一个终端运行 Agent：

```powershell
$env:PINGLAKE_HUB_URL='http://127.0.0.1:8090'
$env:PINGLAKE_ENROLLMENT_TOKEN='development-enrollment-token'
cargo run -p pinglake-agent -- `
  --state-dir .\data\agent-dev
```

## 升级、备份和回滚

升级顺序必须是 Hub-first：先备份 SQLite，再升级 Hub、确认 schema 迁移和 API 健康，之后分批升级 Agent。schema v5 增加监控 JSON、事件表、配置版本、去重索引和告警对象；schema v6 增加 canonical DNS、进程和本机端口样本及索引。v4 → v5 → v6 的迁移会保留已有数据。

在线备份必须使用 SQLite online backup API，或停 Hub 后复制包含已提交 WAL 内容的数据库。只复制 `.db` 而忽略 `.db-wal` 不能作为一致性备份。回滚到 v5 或更早版本需要兼容的旧数据库备份和旧二进制；不能只替换 Hub 二进制后继续使用已经迁移到 v6 的数据库。

详细的升级、WAL 备份、S4 Hub 和 S1–S4 Agent 分批步骤见 [v0.3.0-upgrade.md](docs/v0.3.0-upgrade.md)。生产发布和 S4/S1–S4 更新在当前文档编写时仍为 pending，不能视为已完成。

## 常用命令

更新（保留数据 volume）：

```powershell
git pull
docker compose -f docker-compose.yml -f deploy/docker-compose.tls.yml up -d --build
```

查看状态与日志：

```powershell
docker compose ps
docker compose logs --tail=200 hub caddy
```

不要使用 `docker compose down -v`，否则会删除 SQLite 数据 volume。

## 安全边界

- Agent 只采集本机性能数据，不接收或执行远程命令。
- 跨公网必须使用 HTTPS；`--insecure-skip-verify` 只用于隔离测试环境。
- 不要在二进制下载 URL 中放置长期凭据。远程安装前核对发布包的 `SHA256SUMS.txt`，安装器会拒绝没有显式 SHA-256 的 HTTPS 下载。
- 不要把 `.env`、Agent 配置、数据库或注册令牌提交到 Git。
- Hub 的 `8090` 端口应仅绑定回环地址或由防火墙限制，公网入口只保留反向代理的 `443`。
- Webhook 接收端应验证来源，并避免在 URL 中放置可长期复用的高权限凭证。
