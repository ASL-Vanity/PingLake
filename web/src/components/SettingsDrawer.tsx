import { FormEvent, useEffect, useRef, useState } from "react";
import type { AlertSettings, NodeSnapshot } from "../types";
import { AppIcon } from "./AppIcon";

interface SettingsDrawerProps {
  open: boolean;
  settings: AlertSettings | null;
  nodes: NodeSnapshot[];
  onClose: () => void;
  onSave: (settings: AlertSettings) => Promise<AlertSettings>;
}

export function SettingsDrawer({ open, settings, nodes, onClose, onSave }: SettingsDrawerProps) {
  const [draft, setDraft] = useState<AlertSettings | null>(settings);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const wasOpen = useRef(false);
  const drawerRef = useRef<HTMLElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (open && !wasOpen.current) {
      setDraft(settings ? { ...settings } : null);
      setError(null);
      setSaved(false);
    } else if (open && draft == null && settings) {
      setDraft({ ...settings });
    }
    wasOpen.current = open;
  }, [draft, open, settings]);

  useEffect(() => {
    if (!open) return;
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const focusTimer = window.setTimeout(() => closeButtonRef.current?.focus(), 0);
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(
        drawerRef.current?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      ).filter((element) => !element.hasAttribute("disabled") && element.getClientRects().length > 0);
      if (focusable.length === 0) {
        event.preventDefault();
        drawerRef.current?.focus();
        return;
      }
      const first = focusable[0]!;
      const last = focusable[focusable.length - 1]!;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => {
      window.clearTimeout(focusTimer);
      window.removeEventListener("keydown", handleKey);
      document.body.style.overflow = previousOverflow;
      previousFocus.current?.focus();
    };
  }, [open]);

  if (!open) return null;

  const temperatureAvailable = nodes.some((node) => node.latest?.temperature_celsius != null);

  const changeNumber = (field: keyof AlertSettings, value: string) => {
    const parsed = Number(value);
    setDraft((current) => current ? { ...current, [field]: Number.isFinite(parsed) ? parsed : 0 } : current);
    setSaved(false);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!draft || saving) return;
    const validationError = validateSettings(draft);
    if (validationError) {
      setError(validationError);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const result = await onSave(draft);
      setDraft({ ...result });
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2_000);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "保存设置失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="drawer-layer" role="presentation">
      <button className="drawer-backdrop" type="button" onClick={onClose} aria-label="关闭设置" tabIndex={-1} />
      <aside ref={drawerRef} className="settings-drawer" role="dialog" aria-modal="true" aria-labelledby="settings-title" aria-describedby="settings-description" tabIndex={-1}>
        <header className="drawer-header">
          <div><AppIcon name="settings" size={18} /><div><h2 id="settings-title">告警设置</h2><span id="settings-description">全局阈值与通知</span></div></div>
          <button ref={closeButtonRef} type="button" className="icon-button" onClick={onClose} title="关闭" aria-label="关闭设置"><AppIcon name="close" size={18} /></button>
        </header>
        {!draft ? (
          <div className="drawer-loading"><AppIcon name="loader" className="spin" size={22} />正在载入设置</div>
        ) : (
          <form className="settings-form" onSubmit={submit}>
            <fieldset>
              <legend>告警项目</legend>
              <div className="metric-toggle-grid">
                <ToggleField label="离线" detail="主机停止上报" checked={draft.offline_enabled} onChange={(checked) => { setDraft({ ...draft, offline_enabled: checked }); setSaved(false); }} />
                <ToggleField label="CPU" detail="CPU 使用率阈值" checked={draft.cpu_enabled} onChange={(checked) => { setDraft({ ...draft, cpu_enabled: checked }); setSaved(false); }} />
                <ToggleField label="内存" detail="内存使用率阈值" checked={draft.memory_enabled} onChange={(checked) => { setDraft({ ...draft, memory_enabled: checked }); setSaved(false); }} />
                <ToggleField label="磁盘" detail="磁盘使用率阈值" checked={draft.disk_enabled} onChange={(checked) => { setDraft({ ...draft, disk_enabled: checked }); setSaved(false); }} />
                <ToggleField label="温度" detail={temperatureAvailable ? "已检测到温度传感器" : "当前没有主机上报温度"} checked={draft.temperature_enabled} disabled={!temperatureAvailable} onChange={(checked) => { setDraft({ ...draft, temperature_enabled: checked }); setSaved(false); }} />
              </div>
            </fieldset>
            <fieldset>
              <legend>资源阈值</legend>
              <div className="settings-grid">
                <NumberField label="CPU" suffix="%" value={draft.cpu_percent} onChange={(value) => changeNumber("cpu_percent", value)} min={1} max={100} step={1} />
                <NumberField label="内存" suffix="%" value={draft.memory_percent} onChange={(value) => changeNumber("memory_percent", value)} min={1} max={100} step={1} />
                <NumberField label="磁盘" suffix="%" value={draft.disk_percent} onChange={(value) => changeNumber("disk_percent", value)} min={1} max={100} step={1} />
                <NumberField label="温度" suffix="°C" value={draft.temperature_celsius} onChange={(value) => changeNumber("temperature_celsius", value)} min={-100} max={250} step={1} />
              </div>
            </fieldset>

            <fieldset>
              <legend>触发条件</legend>
              <div className="settings-grid">
                <NumberField label="离线判定" suffix="秒" value={draft.offline_after_seconds} onChange={(value) => changeNumber("offline_after_seconds", value)} min={5} max={86400} step={5} />
                <NumberField label="持续时间" suffix="秒" value={draft.sustained_for_seconds} onChange={(value) => changeNumber("sustained_for_seconds", value)} min={0} max={86400} step={5} />
              </div>
            </fieldset>

            <fieldset>
              <legend>Webhook</legend>
              <label className="toggle-row">
                <span><strong>启用 Webhook</strong><small>活动与恢复事件都会发送</small></span>
                <input
                  type="checkbox"
                  checked={draft.webhook_enabled}
                  onChange={(event) => {
                    setDraft({ ...draft, webhook_enabled: event.target.checked });
                    setSaved(false);
                  }}
                />
                <i aria-hidden="true" />
              </label>
              <label className="field-label webhook-field">
                <span>Webhook URL</span>
                <input
                  type="url"
                  value={draft.webhook_url}
                  onChange={(event) => {
                    setDraft({ ...draft, webhook_url: event.target.value });
                    setSaved(false);
                  }}
                  disabled={!draft.webhook_enabled}
                  placeholder="https://example.com/webhook"
                  autoComplete="url"
                />
              </label>
            </fieldset>

            <fieldset>
              <legend>邮件通知</legend>
              <label className="toggle-row">
                <span><strong>启用邮件通知</strong><small>SMTP 密码仅由 Hub 环境变量读取</small></span>
                <input type="checkbox" checked={draft.email_enabled} onChange={(event) => { setDraft({ ...draft, email_enabled: event.target.checked }); setSaved(false); }} />
                <i aria-hidden="true" />
              </label>
              <label className="field-label webhook-field">
                <span>收件邮箱</span>
                <input
                  type="text"
                  value={draft.email_recipients.join(", ")}
                  onChange={(event) => {
                    const recipients = event.target.value.split(/[,;\n]/).map((value) => value.trim()).filter(Boolean);
                    setDraft({ ...draft, email_recipients: recipients });
                    setSaved(false);
                  }}
                  disabled={!draft.email_enabled}
                  placeholder="ops@example.com, admin@example.com"
                  autoComplete="email"
                />
              </label>
            </fieldset>

            {error && <div className="form-error" role="alert">{error}</div>}
            <footer className="drawer-footer">
              {saved && <span className="save-confirmation">设置已保存</span>}
              <button type="submit" className="primary-button" disabled={saving}>
                {saving ? <AppIcon name="loader" className="spin" size={16} /> : <AppIcon name="save" size={16} />}
                {saving ? "正在保存" : "保存设置"}
              </button>
            </footer>
          </form>
        )}
      </aside>
    </div>
  );
}

function NumberField({
  label,
  suffix,
  value,
  onChange,
  min,
  max,
  step,
}: {
  label: string;
  suffix: string;
  value: number;
  onChange: (value: string) => void;
  min: number;
  max: number;
  step: number;
}) {
  return (
    <label className="field-label">
      <span>{label}</span>
      <div className="number-field">
        <input type="number" value={value} onChange={(event) => onChange(event.target.value)} min={min} max={max} step={step} required />
        <span>{suffix}</span>
      </div>
    </label>
  );
}

function ToggleField({
  label,
  detail,
  checked,
  disabled = false,
  onChange,
}: {
  label: string;
  detail: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="toggle-row compact-toggle">
      <span><strong>{label}</strong><small>{detail}</small></span>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
      <i aria-hidden="true" />
    </label>
  );
}

function validateSettings(settings: AlertSettings): string | null {
  const percents = [settings.cpu_percent, settings.memory_percent, settings.disk_percent];
  const values = [
    ...percents,
    settings.temperature_celsius,
    settings.offline_after_seconds,
    settings.sustained_for_seconds,
  ];
  if (values.some((value) => !Number.isFinite(value))) return "请填写有效的数值";
  if (percents.some((value) => value < 1 || value > 100)) return "资源阈值必须在 1 到 100 之间";
  if (settings.temperature_celsius < -100 || settings.temperature_celsius > 250) return "温度阈值必须在 -100 到 250 °C 之间";
  if (settings.offline_after_seconds < 5 || settings.offline_after_seconds > 86400) return "离线判定必须在 5 到 86400 秒之间";
  if (settings.sustained_for_seconds < 0 || settings.sustained_for_seconds > 86400) return "持续时间必须在 0 到 86400 秒之间";
  const webhookError = validateWebhookUrl(settings.webhook_url, settings.webhook_enabled);
  if (webhookError) return webhookError;
  if (settings.email_enabled && settings.email_recipients.length === 0) return "启用邮件通知时至少添加一个收件邮箱";
  if (settings.email_recipients.some((value) => !isValidEmail(value))) return "收件邮箱格式无效";
  return null;
}

function isValidEmail(value: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);
}

function validateWebhookUrl(value: string, enabled: boolean): string | null {
  const trimmed = value.trim();
  if (!trimmed) return enabled ? "启用 Webhook 时必须填写有效的 HTTP(S) URL" : null;

  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return "Webhook URL 必须是有效的 HTTP(S) URL";
  }
  if ((url.protocol !== "https:" && url.protocol !== "http:") || !url.hostname) {
    return "Webhook URL 必须是有效的 HTTP(S) URL";
  }
  if (url.username || url.password) return "Webhook URL 不能包含内嵌凭据";
  if (isPrivateOrReservedIpLiteral(url.hostname)) {
    return "Webhook URL 不能指向私有或保留地址";
  }
  return null;
}

function isPrivateOrReservedIpLiteral(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/g, "");
  const ipv4 = parseIpv4(host);
  if (ipv4) return !isPublicIpv4(ipv4);
  const ipv6 = parseIpv6(host);
  return ipv6 ? !isPublicIpv6(ipv6) : false;
}

type Ipv4Octets = [number, number, number, number];

function parseIpv4(value: string): Ipv4Octets | null {
  const parts = value.split(".");
  if (parts.length !== 4 || parts.some((part) => !/^\d+$/.test(part))) return null;
  const octets = parts.map(Number);
  if (!octets.every((octet) => octet >= 0 && octet <= 255)) return null;
  return [octets[0]!, octets[1]!, octets[2]!, octets[3]!];
}

function isPublicIpv4([first, second, third, fourth]: Ipv4Octets): boolean {
  if (
    first === 0
    || first === 10
    || first === 127
    || first >= 224
    || (first === 100 && second >= 64 && second <= 127)
    || (first === 169 && second === 254)
    || (first === 172 && second >= 16 && second <= 31)
    || (first === 192 && second === 168)
    || (first === 192 && second === 0 && [0, 2].includes(third))
    || (first === 192 && second === 88 && third === 99)
    || (first === 198 && second >= 18 && second <= 19)
    || (first === 198 && second === 51 && third === 100)
    || (first === 203 && second === 0 && third === 113)
    || (first === 255 && second === 255 && third === 255 && fourth === 255)
  ) {
    return false;
  }
  return true;
}

function parseIpv6(value: string): number[] | null {
  let normalized = value.toLowerCase();
  if (!normalized || normalized.includes("%")) return null;

  if (normalized.includes(".")) {
    const boundary = normalized.lastIndexOf(":");
    if (boundary < 0) return null;
    const ipv4 = parseIpv4(normalized.slice(boundary + 1));
    if (!ipv4) return null;
    const firstGroup = ((ipv4[0]! << 8) | ipv4[1]!).toString(16);
    const secondGroup = ((ipv4[2]! << 8) | ipv4[3]!).toString(16);
    normalized = `${normalized.slice(0, boundary)}:${firstGroup}:${secondGroup}`;
  }

  const doubleColonParts = normalized.split("::");
  if (doubleColonParts.length > 2) return null;
  const left = doubleColonParts[0] ? doubleColonParts[0].split(":") : [];
  const right = doubleColonParts.length === 2 && doubleColonParts[1] ? doubleColonParts[1].split(":") : [];
  const groups = [...left, ...right];
  if (groups.some((group) => !/^[0-9a-f]{1,4}$/.test(group))) return null;
  const missing = 8 - groups.length;
  if ((doubleColonParts.length === 1 && missing !== 0) || missing < 0) return null;
  const expanded = [...left, ...Array(missing).fill("0"), ...right].map((group) => Number.parseInt(group, 16));
  if (expanded.length !== 8) return null;
  return expanded.flatMap((group) => [group >> 8, group & 0xff]);
}

function isPublicIpv6(bytes: number[]): boolean {
  if (bytes.length !== 16) return false;
  const ipv4Mapped = bytes.slice(0, 10).every((byte) => byte === 0) && bytes[10] === 0xff && bytes[11] === 0xff;
  if (ipv4Mapped) return isPublicIpv4([bytes[12]!, bytes[13]!, bytes[14]!, bytes[15]!]);
  const unspecified = bytes.every((byte) => byte === 0);
  const loopback = bytes.slice(0, 15).every((byte) => byte === 0) && bytes[15] === 1;
  const multicast = bytes[0]! === 0xff;
  const uniqueLocal = (bytes[0]! & 0xfe) === 0xfc;
  const linkLocal = bytes[0]! === 0xfe && (bytes[1]! & 0xc0) === 0x80;
  if (unspecified || loopback || multicast || uniqueLocal || linkLocal) return false;

  const globalUnicast = (bytes[0]! & 0xe0) === 0x20;
  const documentation = [0x20, 0x01, 0x0d, 0xb8].every((byte, index) => bytes[index] === byte);
  const nat64WellKnown = [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0].every((byte, index) => bytes[index] === byte);
  const nat64Local = [0x00, 0x64, 0xff, 0x9b, 0x00, 0x01].every((byte, index) => bytes[index] === byte);
  return globalUnicast && !documentation && !nat64WellKnown && !nat64Local;
}
