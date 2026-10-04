# PingLake v0.3.0 中文使用手册

这份手册面向第一次部署 PingLake 的管理员，也适用于从 v0.1.x 或 v0.2.0 升级的实例。PingLake 由 Hub、Agent 和 Web 控制台组成：Hub 保存监控数据并提供控制台，Agent 主动向 Hub 上报本机指标，浏览器访问 Hub 提供的 Web 页面。

v0.3.0 同时包含新版控制台和扩展监控协议。新版控制台使用紧凑导航、主题按钮、节点卡片/列表视图、节点详情页签和移动端抽屉；扩展监控提供服务、探测、进程、本机端口、质量和浏览器到节点延迟能力。

## 1. 部署前准备

生产环境准备一台运行 Docker Compose 的 Hub 主机，并准备一个指向该主机的域名。公网部署必须使用 HTTPS；被监控的 Windows/Linux 主机只需要能够主动访问 Hub，不需要开放入站端口。

从 GitHub 的 [Releases](https://github.com/ASL-Vanity/PingLake/releases) 页面下载与版本匹配的 Agent 和安装脚本。每次下载二进制后先核对同一发布包里的 `SHA256SUMS.txt`。

部署目录至少需要以下文件：

```text
docker-compose.yml
deploy/docker-compose.tls.yml
.env.example
```

## 2. 首次部署 Hub

复制环境变量模板：

```powershell
Copy-Item .env.example .env
```

编辑 `.env`，至少设置两个长度不小于 16 个字节的随机值：

```dotenv
PINGLAKE_ADMIN_PASSWORD=替换为管理员密码
PINGLAKE_ENROLLMENT_TOKEN=替换为Agent注册令牌
PINGLAKE_DOMAIN=monitor.example.com
```

PingLake 没有内置默认密码。登录页只有管理员密码字段；密码就是 `PINGLAKE_ADMIN_PASSWORD` 的值。`PINGLAKE_ENROLLMENT_TOKEN` 只用于新 Agent 注册，不能用于登录。

确认域名已经解析到 Hub 主机后，启动 HTTPS 部署：

```powershell
docker compose -f docker-compose.yml -f deploy/docker-compose.tls.yml up -d --build
```

访问 `https://monitor.example.com`。首次启动时 Caddy 会申请证书，Hub 数据保存在 Docker volume `pinglake-data` 中。

### 可信内网临时部署

没有域名或只在隔离内网中检查功能时，可以使用 IP 模式：

```dotenv
PINGLAKE_PUBLISH_ADDR=0.0.0.0
PINGLAKE_COOKIE_SECURE=false
```

```powershell
docker compose up -d --build hub
```

然后访问 `http://Hub-IP:8090`。这个模式会以明文 HTTP 传输会话、注册令牌、Agent 凭据和指标，不能用于公网或不可信网络。

## 3. 接入监控节点

所有 Agent 都使用同一个 Hub URL 和同一个注册令牌。节点名称只用于控制台显示，不要求节点拥有域名。

### Linux

在目标 Linux 主机上以 root 运行：

```bash
sudo ./install-agent-linux.sh \
  --binary ./pinglake-agent-linux-amd64 \
  --hub-url https://monitor.example.com \
  --name vm-linux-01
```

安装器会隐藏读取注册令牌，并创建 `pinglake-agent` systemd 服务。也可以使用仅 root 可读的令牌文件：

```bash
sudo ./install-agent-linux.sh \
  --binary ./pinglake-agent-linux-amd64 \
  --hub-url https://monitor.example.com \
  --enrollment-token-file /root/pinglake-enrollment-token \
  --name vm-linux-01
```

如果直接从 HTTPS 下载二进制，必须同时提供发布清单里的 SHA-256：

```bash
sudo ./install-agent-linux.sh \
  --binary https://downloads.example.com/pinglake-agent-linux-amd64 \
  --sha256 SHA256值 \
  --hub-url https://monitor.example.com \
  --name vm-linux-01
```

查看服务状态：

```bash
sudo systemctl status pinglake-agent
sudo journalctl -u pinglake-agent -n 100 --no-pager
```

### Windows

在管理员 PowerShell 中运行：

```powershell
.\install-agent-windows.ps1 `
  -Binary .\pinglake-agent.exe `
  -HubUrl https://monitor.example.com `
  -Name vm-windows-01
```

脚本会安全读取注册令牌，并将配置保存在 `C:\ProgramData\PingLake\agent.json`，服务名称为 `PingLakeAgent`。如果二进制来自 HTTPS URL，使用 `-Sha256` 传入发布清单中的值。

查看服务：

```powershell
Get-Service PingLakeAgent
Get-Content C:\ProgramData\PingLake\pinglake-agent.log -Tail 100 -Wait
```

节点注册成功后会出现在“运行概览”的受监主机区域。所有节点完成注册后，应在 `.env` 中更换 `PINGLAKE_ENROLLMENT_TOKEN` 并重启 Hub；已注册节点不受影响，新节点需要使用新令牌。

## 4. 控制台操作

### 运行概览

概览页显示在线节点、离线节点、活动告警以及在线节点的 CPU、内存平均值。节点卡片默认以卡片视图显示；节点较多时可以切换列表视图。状态筛选支持全部、在线和离线，另外可以按分组和主机名搜索。

点击节点卡片或列表行可以打开详情页。详情页包含当前 CPU、内存、磁盘、网络、Hub 延迟、最近心跳、系统信息、磁盘和进程数据，并提供 1 小时、6 小时、24 小时历史范围切换。

没有采样时界面显示“暂无采样”；真实值为 0 时会显示 `0%`。离线节点的历史图表表示最后一次上报数据。

### 分组

在节点区域的“新建分组”输入名称，按 Enter 或点击创建按钮。创建后可在每张节点卡片或列表行底部选择分组。分组筛选只影响当前视图，不会删除节点。

### 告警中心

告警中心支持活动、全部和已恢复筛选，也支持按节点和告警类型搜索。点击告警事件会跳转到对应节点详情。告警设置中的阈值、持续时间和启用状态会在 Hub 端校验并保存。

### 主题和视图偏好

侧栏或顶部的主题按钮可以切换：

- `system`：跟随操作系统；深色系统解析为 Obsidian，浅色系统解析为 Porcelain。
- `obsidian`：深石墨深色主题。
- `porcelain`：微暖浅色主题。
- `lagoon`：深蓝绿色主题。
- `amber`：温暖米白主题。

主题偏好保存在浏览器的 `localStorage["pinglake.theme"]`。旧版本的 `light`、`dark`、`midnight`、`circuit` 会在首次读取时迁移。节点卡片/列表视图保存在 `localStorage["pinglake.node-view"]`。

### 告警设置

“告警设置”抽屉负责 CPU、内存、磁盘、温度和离线判定，以及 Webhook 和邮件通知。保存时会保留字段校验、邮箱格式检查和 Webhook 公网地址限制。主题不在告警设置中管理，而是在导航栏主题按钮中管理。

## 5. 配置扩展监控

打开节点详情的“监测配置”页签。Hub 为每次保存分配 revision，Agent 每 15 秒轮询并回报已应用 revision。页面会区分 Hub 已保存、Agent 正在等待、Agent 已应用和 Agent 报错四种状态。

### 服务检查

服务名必须是 Linux systemd unit 名称或 Windows SCM service 名称，不是显示名称、路径或 shell 命令。预期状态可选 running 或 stopped。服务历史记录按检查完成时间保存，未知或权限不足不会被当成 stopped。

### 主动探测

每个节点最多配置 32 个探测，间隔 10–86,400 秒，默认 30 秒；超时 1–10,000 毫秒且必须小于间隔。四类探测的含义不同：

- **ICMP**：发送一个 echo 请求并测量往返时间。
- **TCP**：向目标主机和端口建立连接。
- **HTTP(S)**：测量响应头时间；默认期望 2xx，启用内容匹配时最多读取 64 KiB，TLS 校验保持开启。
- **DNS**：从 Agent 的解析器查询 A 或 AAAA 记录，不连接返回的地址；可选地精确匹配某个 IP。结果保留 DNS RCODE 和答案。

私网和 loopback 目标默认被策略拒绝。只有明确授权的 Agent 才能在本地配置中设置：

```toml
allow_private_probe_targets = true
allow_loopback_probe_targets = true
```

云元数据、链路本地、未指定和组播地址仍然禁止。探测不携带 Hub/Agent 凭据，也不跟随重定向。

### 进程和本机端口检查

进程检查是只读的精确进程名匹配，不接受 shell 命令、glob 或正则表达式。零个进程是一次成功采集到的真实结果；权限不足、未支持或过期是未知状态。本机端口检查只读 Agent 当前网络命名空间的 TCP/UDP socket 表，不主动连接端口；可配置协议、地址族、地址范围和 1–65,535 端口。

### 资源、网络和 TCP

资源页签显示每核心 CPU、CPU user/system/IOWait、内存、Swap、磁盘 IO、inode、网卡错误/丢弃和 TCP 状态。Linux CPU IOWait 是等待 CPU 时间，磁盘读取/写入延迟是已完成请求的平均值，两者不能互相替代。容器中的 TCP 表只描述 Agent 所在网络命名空间，不一定代表宿主机。

首次速率样本、计数器重置、权限不足和不支持值显示为 warming-up/unknown，而不是 0。原始累计计数以十进制字符串保存，避免丢失 u64 精度。

### 历史和质量

资源历史最多返回 240 个时间桶代表点，受 4 MiB 响应限制；选择具体设备或指标可以减少响应。服务、探测、进程和本机端口历史使用独立去重事件表。质量页签区分采集时间、Hub 接收时间、样本年龄、采集/发送耗时和 Agent→Hub 延迟。

上传成功率以最近 100 次尝试计算，失败不会伪装成零延迟；覆盖率按版本化调度槽计算，缺失、离线、暂停和 unsupported 保持 unknown。探测统计使用成功原始时长的 nearest-rank P50/P95/P99，并展示成功率、已观测可用率、覆盖率和未知时长。修改目标定义后，统计不会混合不相关的旧样本。

## 6. 浏览器到节点 HTTPS 测点

这是访问者浏览器直接请求节点 HTTPS URL 的 HTTP 应用往返时间，不是 ICMP、单向延迟或 Agent→Hub 上传耗时。每个节点需要一个独立可访问的 HTTPS URL；NAT-only 节点显示不可用。

Agent 可选地监听本地空响应端点：

```toml
latency_bind = "127.0.0.1:18091"
dashboard_origin = "https://monitor.example.com"
```

端点必须放在已有 HTTPS 反向代理后面，例如：

```caddyfile
node01.example.com {
    handle /pinglake/latency {
        reverse_proxy 127.0.0.1:18091
    }
}
```

将节点 URL 设置为 `https://node01.example.com/pinglake/latency`，关闭该路径的 CDN/反向代理缓存并保留 `Cache-Control`。端点只返回空的 204，不返回凭据或指标；仅允许配置的 dashboard Origin，拒绝其他来源，并限制每秒请求数。新增测点来源后需刷新控制台以取得更新后的 CSP。真实 TLS、CORS、CSP、缓存和浏览器网络权限必须在实际部署中验收。

## 7. Agent spool、协议兼容和数据保存

Agent 的采集和上传独立运行。待发送报告最多保留 64 条，并持久化到 Agent state 目录；队列满时丢弃最旧报告，超过 300 秒的报告过期。队列损坏或过大时会记录错误并从空队列启动，主机采集继续进行。

注册时 Hub 宣布最高 monitoring schema：

- v2 Agent → v2 Hub：上传 DNS、进程和本机端口等扩展结果。
- v2 Agent → v1 Hub：自动投影为 v1，只保留旧 Hub 能理解的配置和结果；v2-only 状态不会伪装成健康。
- v1 Agent → v2 Hub：Hub 继续接受原有资源、服务和探测报告。

报告身份由节点、monitoring session 和 sample sequence 组成。同一身份的完全相同重试是幂等成功；同一身份但内容不同会返回 conflict，Hub 不会静默覆盖第一份数据。原始数据默认保存 7 天，Hub 是已接受报告的事实来源。

## 8. 配置 SMTP 邮件通知

SMTP 发件账号由 Hub 的 `.env` 配置，收件人由控制台“告警设置 → 邮件通知”配置。PingLake 使用加密 SMTP relay，生产环境优先使用服务商提供的 465 端口。邮箱服务商通常要求使用 SMTP 授权码或应用专用密码，不能直接填写网页登录密码。

示例：

```dotenv
PINGLAKE_SMTP_HOST=smtp.example.com
PINGLAKE_SMTP_PORT=465
PINGLAKE_SMTP_FROM=pinglake-alert@example.com
PINGLAKE_SMTP_USERNAME=pinglake-alert@example.com
PINGLAKE_SMTP_PASSWORD=SMTP授权码
```

如果配置了 SMTP 主机，主机、端口、发件地址必须同时存在；用户名和密码也必须成对存在。`PINGLAKE_SMTP_PASSWORD` 只放在 Hub 主机的 `.env`，不要提交到 GitHub。

改完 `.env` 后重新创建 Hub：

```powershell
docker compose up -d --force-recreate hub
```

然后在控制台启用邮件通知，并在收件邮箱中填写一个或多个地址：

```text
ops@example.com, admin@example.com
```

逗号、分号和换行都可以分隔地址，最多 32 个。邮件会发送活动告警和恢复事件。当前版本没有独立的测试邮件按钮；可以通过临时降低测试环境阈值触发事件，并配合 Hub 日志验证投递：

```powershell
docker compose logs -f hub
```

## 9. Webhook 通知

在告警设置中启用 Webhook，并填写 `https://` 地址。Hub 会拒绝带内嵌用户名/密码的 URL，并拒绝指向本机、内网和保留地址的目标。接收端应校验请求来源，并避免把长期高权限凭据放入 URL。

## 10. 更新、数据库迁移和回滚

更新前先备份 `.env` 和数据库。升级顺序必须是 Hub-first：先备份 SQLite，再升级 Hub、确认 schema 迁移和 API 健康，之后分批升级 Agent。生产更新通常在项目目录执行：

```powershell
git pull
docker compose -f docker-compose.yml -f deploy/docker-compose.tls.yml up -d --build
```

Compose 会保留现有 `pinglake-data` volume。不要使用下面的命令进行普通升级：

```powershell
docker compose down -v
```

`-v` 会删除 SQLite 数据 volume。

Hub 数据库位于容器内的 `/data/pinglake.db`。可以在停机后使用临时容器把 volume 备份到当前目录：

```powershell
$volume = docker volume ls `
  --filter "label=com.docker.compose.project=pinglake" `
  --filter "label=com.docker.compose.volume=pinglake-data" `
  --format "{{.Name}}"
if ([string]::IsNullOrWhiteSpace($volume)) { throw "pinglake-data volume not found" }
docker compose stop hub
docker run --rm `
  -v "${volume}:/data:ro" `
  -v "${PWD}:/backup" `
  alpine:3.20 `
  tar czf /backup/pinglake-data-$(Get-Date -Format yyyyMMdd-HHmmss).tar.gz -C /data .
docker compose start hub
```

Compose 会为 volume 自动添加项目名前缀，所以上面的命令先按 Compose 标签解析真实名称。如果项目名或 volume 标签被手动改过，可用 `docker volume ls` 确认名称。恢复前停止 Hub，解压备份覆盖 volume 内容，再启动 Hub。

每个 GitHub Release 的 Agent 二进制都附带 `SHA256SUMS.txt`。升级 Agent 时先下载、核对 SHA-256，再运行对应平台安装器；安装器会原子替换二进制和配置并尝试恢复旧服务。

### schema v4 → v5 → v6

- **v5**：增加监控 JSON、服务/探测事件表、配置版本、去重索引和告警对象字段。
- **v6**：增加 canonical DNS、进程和本机端口样本，以及对应索引。

迁移会保留已有节点、历史和告警。在线备份必须使用 SQLite online backup API，或停 Hub 后复制包含已提交 WAL 内容的数据库；只复制 `.db` 而忽略 `.db-wal` 不能作为一致性备份。

Hub 已迁移到 v6 后，不能只替换为旧二进制并继续使用 v6 数据库。回滚 v5 或更早版本必须同时恢复兼容的旧数据库备份和旧二进制。Agent 回滚应使用保存的旧二进制并保留原 state 目录和凭据。

v0.3.0 的 S4 Hub 以及 S1–S4 Agent 生产更新在当前文档发布时仍是 **pending**，不能把本地测试结果当成生产部署已完成。

## 11. 常见问题

### 页面打不开

检查容器和 Caddy 状态：

```powershell
docker compose ps
docker compose logs --tail=200 hub caddy
```

确认 DNS、TCP 80/443、UDP 443 和防火墙规则。IP 模式则确认 `PINGLAKE_PUBLISH_ADDR` 和 `PINGLAKE_PUBLISH_PORT`。

### 登录失败

确认输入的是当前 `.env` 中的 `PINGLAKE_ADMIN_PASSWORD`，而不是 Agent 注册令牌或 SMTP 授权码。修改管理员密码后必须重启 Hub；系统不会在页面中显示或找回密码。

### 节点没有出现

确认 Agent 的 Hub URL 使用正确协议和端口，注册令牌仍然有效，并查看 Agent 服务日志。公网环境必须使用 HTTPS；只有隔离测试环境才可以显式启用 `--allow-insecure-http` 或 `-AllowInsecureHttp`。

### 邮件没有发送

确认 SMTP 主机、465 端口、发件地址、用户名和授权码已经写入 Hub 的 `.env`，并且重启了 Hub。确认页面中已经启用邮件通知且至少填写一个有效收件地址，然后查看 `docker compose logs hub` 中的投递错误。

### 浏览器仍显示旧 Logo 或旧页面

先执行强制刷新（Windows/Linux：`Ctrl+Shift+R`），或者关闭旧标签页后重新打开。浏览器会缓存 favicon 和 Apple Touch Icon；v0.3.0 已为新 BrandMark 提供带版本参数的资源地址。

## 12. 安全检查清单

- 生产环境使用 HTTPS，不把 Hub 的 8090 端口直接暴露到公网。
- 管理员密码和 Agent 注册令牌使用随机长值，且不要提交 `.env`。
- Agent 注册完成后轮换 `PINGLAKE_ENROLLMENT_TOKEN`。
- 下载 Agent 后核对 `SHA256SUMS.txt`。
- SMTP 密码使用授权码或应用专用密码，并只保存在 Hub 环境变量中。
- 定期备份 `pinglake-data` volume 和 `.env`，并验证恢复流程。
- 不要使用 `docker compose down -v` 作为普通升级命令。
