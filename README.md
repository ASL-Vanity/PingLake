# PingLake

PingLake 是面向少量到中等规模 Windows/Linux 主机的实时状态监控系统。它由一个中心 Hub、每台主机上的只读 Agent 和同源 Web 控制台组成。

Agent 主动连接 Hub，因此被监控节点不需要域名、固定公网地址或入站端口。只有 Hub 需要被这些节点访问；跨公网部署时必须使用 HTTPS。

## 功能

- Windows 与 Linux 统一 Agent
- CPU、内存、Swap、磁盘、网络、温度、负载、进程数和运行时间
- 每核心 CPU、可用内存、磁盘 IO/IOPS、inode、网卡错误/丢弃、TCP 状态及平台能力标记
- 指定 systemd/Windows 服务、ICMP/TCP/HTTP 主动探测和 Agent 上报质量
- 探测 P50/P95/P99、成功率、监测覆盖率和未知时长
- 分组创建与删除；删除分组保留节点和历史
- 浏览器到每台主机独立 HTTPS 测点的访问延迟（需配置测点）
- 实时状态推送与 1/6/24 小时历史图表
- CPU、内存、磁盘、温度和离线告警
- 通用 JSON Webhook 通知
- SMTP 邮件通知与按服务/探测对象分别去重的告警
- SQLite 单文件存储，默认保留 7 天原始数据
- Agent 密钥哈希存储、管理员会话、只读采集
- Docker Hub 部署与 Windows/Linux 系统服务安装脚本

新增指标的平台差异、测点配置、数据口径与数据库迁移见 [监测扩展说明](docs/monitoring-guide.md)。

## 快速启动 Hub

1. 复制 `.env.example` 为 `.env`，替换两个密码字段并设置 `PINGLAKE_DOMAIN`。
2. 将域名 A/AAAA 记录指向 Hub 云服务器，并开放 TCP `80/443` 与 UDP `443`。
3. 启动：

```powershell
docker compose -f docker-compose.yml -f deploy/docker-compose.tls.yml up -d --build
```

访问 `https://你的域名`，使用 `.env` 中的管理员密码登录。

仅限可信内网或临时测试的 IP 模式：

```dotenv
PINGLAKE_PUBLISH_ADDR=0.0.0.0
PINGLAKE_COOKIE_SECURE=false
```

```powershell
docker compose up -d --build hub
```

然后访问 `http://Hub-IP:8090`。不要在公网长期使用明文 HTTP。

基础 Compose 文件不加载 Caddy，因此无域名的 IP 模式不会要求 `PINGLAKE_DOMAIN`。TLS 覆盖文件会在启动前拒绝空域名。Compose 的 `healthcheck` 目前是 Hub API 存活检查，不能证明 SQLite 在磁盘满、只读或运行时 I/O 失败后仍可写入；生产环境仍应对 Hub 建立外部 HTTPS 探测并配置磁盘告警。

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

## 本地开发

```powershell
cd web
npm install
npm run build
cd ..
$env:PINGLAKE_ADMIN_PASSWORD='development-password'
$env:PINGLAKE_ENROLLMENT_TOKEN='development-enrollment-token'
$env:PINGLAKE_ALLOW_WEAK_ADMIN_PASSWORD='true' # Only for isolated local tests.
$env:PINGLAKE_COOKIE_SECURE='false'
cargo run -p pinglake-hub
```

另一个终端运行 Agent：

```powershell
$env:PINGLAKE_HUB_URL='http://127.0.0.1:8090'
$env:PINGLAKE_ENROLLMENT_TOKEN='development-enrollment-token'
cargo run -p pinglake-agent -- `
  --state-dir .\data\agent-dev
```

## 安全边界

- Agent 只采集本机性能数据，不接收或执行远程命令。
- 跨公网必须使用 HTTPS；`--insecure-skip-verify` 只用于隔离测试环境。
- 不要在二进制下载 URL 中放置长期凭据。远程安装前核对发布包的 `SHA256SUMS.txt`，安装器会拒绝没有显式 SHA-256 的 HTTPS 下载。
- 不要把 `.env`、Agent 配置、数据库或注册令牌提交到 Git。
- Hub 的 `8090` 端口应仅绑定回环地址或由防火墙限制，公网入口只保留反向代理的 `443`。
- Webhook 接收端应验证来源，并避免在 URL 中放置可长期复用的高权限凭证。
