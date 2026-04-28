import { useMemo, useState } from "react";
import { AlertTriangle, FilePlus2, X } from "lucide-react";
import * as cmd from "../lib/commands";
import type { SshParams, WebServerKind } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";

// Wizard for spinning up a new site config without touching an AST.
// We compose idiomatic config text from a small form, then write it
// through the existing safe-create pipeline (mkdir → no-clobber check
// → atomic write → optional enable). Apache + Caddy only — nginx
// already has its own creation flow inside NginxPanel.

type Props = {
  kind: Extract<WebServerKind, "apache" | "caddy">;
  sshParams: SshParams;
  onClose: () => void;
  /** Called after a successful create; receives the absolute path of
   *  the new file so the caller can refresh + open it in the editor. */
  onCreated: (path: string) => void;
};

type ApacheForm = {
  fileName: string;
  serverName: string;
  serverAlias: string;
  documentRoot: string;
  port: string;
  ssl: boolean;
  sslCert: string;
  sslKey: string;
  enableAfter: boolean;
};

type CaddyForm = {
  fileName: string;
  address: string;
  mode: "reverse-proxy" | "static";
  upstream: string;
  root: string;
  encode: boolean;
  // Caddy doesn't have a per-file enable like Apache; this just
  // surfaces the import-line reminder in the result.
  enableAfter: boolean;
};

export default function NewWebServerSite({ kind, sshParams, onClose, onCreated }: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const [apache, setApache] = useState<ApacheForm>({
    fileName: "",
    serverName: "",
    serverAlias: "",
    documentRoot: "/var/www/html",
    port: "80",
    ssl: false,
    sslCert: "",
    sslKey: "",
    enableAfter: true,
  });
  const [caddy, setCaddy] = useState<CaddyForm>({
    fileName: "",
    address: "",
    mode: "reverse-proxy",
    upstream: "127.0.0.1:8080",
    root: "/var/www/html",
    encode: true,
    enableAfter: true,
  });

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [warning, setWarning] = useState("");

  // Auto-suggest a file name from the most-meaningful identifier.
  const suggestedFileName = useMemo(() => {
    if (kind === "apache") {
      const base =
        apache.serverName.trim() || sanitizeLeaf(apache.documentRoot);
      return base ? `${sanitizeLeaf(base)}.conf` : "";
    }
    const addr = caddy.address.trim().replace(/^https?:\/\//, "");
    const stem = sanitizeLeaf(addr) || "site";
    return `${stem}.caddyfile`;
  }, [kind, apache.serverName, apache.documentRoot, caddy.address]);

  const fileName =
    kind === "apache"
      ? apache.fileName || suggestedFileName
      : caddy.fileName || suggestedFileName;

  const generated = useMemo(() => {
    if (kind === "apache") return renderApacheVhost(apache);
    return renderCaddySite(caddy);
  }, [kind, apache, caddy]);

  const submit = async () => {
    if (busy) return;
    setError("");
    setWarning("");
    if (!fileName) {
      setError(t("File name is required."));
      return;
    }
    if (kind === "apache" && !apache.serverName.trim()) {
      setError(t("ServerName is required."));
      return;
    }
    if (kind === "caddy" && !caddy.address.trim()) {
      setError(t("Address is required (e.g. example.com or :443)."));
      return;
    }
    if (
      kind === "caddy" &&
      caddy.mode === "reverse-proxy" &&
      !caddy.upstream.trim()
    ) {
      setError(t("Upstream is required for reverse proxy mode."));
      return;
    }
    setBusy(true);
    try {
      const result = await cmd.webServerCreateSite({
        ...sshParams,
        kind,
        leafName: fileName,
        content: generated,
        enableAfter:
          kind === "apache" ? apache.enableAfter : caddy.enableAfter,
      });
      if (kind === "caddy" && result.enableOutput) {
        // Caddy has no per-file enable; surface the import reminder.
        setWarning(result.enableOutput);
      }
      onCreated(result.path);
      onClose();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ws-newdialog__scrim" onClick={onClose}>
      <div
        className="ws-newdialog"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="ws-newdialog__head">
          <span className="ws-newdialog__title">
            <FilePlus2 size={12} />{" "}
            {kind === "apache" ? t("New Apache vhost") : t("New Caddy site")}
          </span>
          <button
            type="button"
            className="ws-newdialog__close"
            onClick={onClose}
            title={t("Close")}
          >
            <X size={12} />
          </button>
        </div>

        <div className="ws-newdialog__body">
          <div className="ws-newdialog__form">
            <Field label={t("File name")} hint={suggestedFileName}>
              <input
                className="ngx-input mono"
                value={kind === "apache" ? apache.fileName : caddy.fileName}
                placeholder={suggestedFileName}
                spellCheck={false}
                onChange={(e) =>
                  kind === "apache"
                    ? setApache({ ...apache, fileName: e.target.value })
                    : setCaddy({ ...caddy, fileName: e.target.value })
                }
              />
            </Field>

            {kind === "apache" ? (
              <ApacheForm form={apache} onChange={setApache} t={t} />
            ) : (
              <CaddyFormFields form={caddy} onChange={setCaddy} t={t} />
            )}
          </div>

          <div className="ws-newdialog__preview">
            <div className="ws-newdialog__preview-head mono">
              {t("Preview")}
            </div>
            <pre className="ws-newdialog__preview-body mono">{generated}</pre>
          </div>
        </div>

        {warning && (
          <div className="ws-newdialog__warn mono">
            <AlertTriangle size={11} /> {warning}
          </div>
        )}
        {error && (
          <div className="ws-newdialog__error mono">
            <AlertTriangle size={11} /> {error}
          </div>
        )}

        <div className="ws-newdialog__actions">
          <button
            type="button"
            className="btn btn--ghost"
            onClick={onClose}
            disabled={busy}
          >
            {t("Cancel")}
          </button>
          <button
            type="button"
            className="btn btn--primary"
            onClick={() => void submit()}
            disabled={busy}
          >
            {busy ? t("Creating…") : t("Create site")}
          </button>
        </div>
      </div>
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="ws-newdialog__field">
      <span className="ws-newdialog__label">
        {label}
        {hint && <span className="ws-newdialog__hint mono">{hint}</span>}
      </span>
      {children}
    </label>
  );
}

function ApacheForm({
  form,
  onChange,
  t,
}: {
  form: ApacheForm;
  onChange: (next: ApacheForm) => void;
  t: (s: string) => string;
}) {
  return (
    <>
      <Field label={t("ServerName")}>
        <input
          className="ngx-input mono"
          value={form.serverName}
          spellCheck={false}
          placeholder="example.com"
          onChange={(e) => onChange({ ...form, serverName: e.target.value })}
        />
      </Field>
      <Field label={t("ServerAlias (optional)")}>
        <input
          className="ngx-input mono"
          value={form.serverAlias}
          spellCheck={false}
          placeholder="www.example.com"
          onChange={(e) => onChange({ ...form, serverAlias: e.target.value })}
        />
      </Field>
      <Field label={t("DocumentRoot")}>
        <input
          className="ngx-input mono"
          value={form.documentRoot}
          spellCheck={false}
          onChange={(e) =>
            onChange({ ...form, documentRoot: e.target.value })
          }
        />
      </Field>
      <Field label={t("Port")}>
        <input
          className="ngx-input mono"
          value={form.port}
          spellCheck={false}
          onChange={(e) => onChange({ ...form, port: e.target.value })}
        />
      </Field>
      <label className="ngx-form__flag">
        <input
          type="checkbox"
          checked={form.ssl}
          onChange={(e) =>
            onChange({
              ...form,
              ssl: e.target.checked,
              port: e.target.checked && form.port === "80" ? "443" : form.port,
            })
          }
        />
        {t("Enable SSL (mod_ssl)")}
      </label>
      {form.ssl && (
        <>
          <Field label={t("SSL certificate path")}>
            <input
              className="ngx-input mono"
              value={form.sslCert}
              spellCheck={false}
              placeholder="/etc/letsencrypt/live/example.com/fullchain.pem"
              onChange={(e) => onChange({ ...form, sslCert: e.target.value })}
            />
          </Field>
          <Field label={t("SSL private key path")}>
            <input
              className="ngx-input mono"
              value={form.sslKey}
              spellCheck={false}
              placeholder="/etc/letsencrypt/live/example.com/privkey.pem"
              onChange={(e) => onChange({ ...form, sslKey: e.target.value })}
            />
          </Field>
        </>
      )}
      <label className="ngx-form__flag">
        <input
          type="checkbox"
          checked={form.enableAfter}
          onChange={(e) => onChange({ ...form, enableAfter: e.target.checked })}
        />
        {t("Enable site after creation (a2ensite)")}
      </label>
    </>
  );
}

function CaddyFormFields({
  form,
  onChange,
  t,
}: {
  form: CaddyForm;
  onChange: (next: CaddyForm) => void;
  t: (s: string) => string;
}) {
  return (
    <>
      <Field
        label={t("Address")}
        hint={t("Hostname (auto-HTTPS) or :port for HTTP-only")}
      >
        <input
          className="ngx-input mono"
          value={form.address}
          spellCheck={false}
          placeholder="example.com"
          onChange={(e) => onChange({ ...form, address: e.target.value })}
        />
      </Field>
      <Field label={t("Mode")}>
        <select
          className="ngx-input mono"
          value={form.mode}
          onChange={(e) =>
            onChange({
              ...form,
              mode: e.target.value as CaddyForm["mode"],
            })
          }
        >
          <option value="reverse-proxy">{t("Reverse proxy")}</option>
          <option value="static">{t("Static file server")}</option>
        </select>
      </Field>
      {form.mode === "reverse-proxy" ? (
        <Field label={t("Upstream")} hint="host:port">
          <input
            className="ngx-input mono"
            value={form.upstream}
            spellCheck={false}
            placeholder="127.0.0.1:8080"
            onChange={(e) => onChange({ ...form, upstream: e.target.value })}
          />
        </Field>
      ) : (
        <Field label={t("Document root")}>
          <input
            className="ngx-input mono"
            value={form.root}
            spellCheck={false}
            onChange={(e) => onChange({ ...form, root: e.target.value })}
          />
        </Field>
      )}
      <label className="ngx-form__flag">
        <input
          type="checkbox"
          checked={form.encode}
          onChange={(e) => onChange({ ...form, encode: e.target.checked })}
        />
        {t("Enable gzip + zstd compression")}
      </label>
      <label className="ngx-form__flag">
        <input
          type="checkbox"
          checked={form.enableAfter}
          onChange={(e) => onChange({ ...form, enableAfter: e.target.checked })}
        />
        {t("Show import-line reminder after creation")}
      </label>
    </>
  );
}

// ── Config text generators ──────────────────────────────────────────

function renderApacheVhost(form: ApacheForm): string {
  const port = form.port.trim() || "80";
  const lines: string[] = [];
  lines.push(`<VirtualHost *:${port}>`);
  lines.push(`    ServerName ${form.serverName.trim() || "example.com"}`);
  if (form.serverAlias.trim()) {
    lines.push(`    ServerAlias ${form.serverAlias.trim()}`);
  }
  lines.push(`    DocumentRoot ${form.documentRoot.trim() || "/var/www/html"}`);
  lines.push("");
  lines.push(`    <Directory ${form.documentRoot.trim() || "/var/www/html"}>`);
  lines.push("        Options Indexes FollowSymLinks");
  lines.push("        AllowOverride All");
  lines.push("        Require all granted");
  lines.push("    </Directory>");
  if (form.ssl) {
    lines.push("");
    lines.push("    SSLEngine on");
    lines.push(
      `    SSLCertificateFile ${
        form.sslCert.trim() || "/etc/ssl/certs/fullchain.pem"
      }`,
    );
    lines.push(
      `    SSLCertificateKeyFile ${
        form.sslKey.trim() || "/etc/ssl/private/privkey.pem"
      }`,
    );
    lines.push("    SSLProtocol all -SSLv3 -TLSv1 -TLSv1.1");
    lines.push("    SSLHonorCipherOrder off");
  }
  lines.push("");
  lines.push(
    `    ErrorLog \${APACHE_LOG_DIR}/${
      form.serverName.trim() || "site"
    }-error.log`,
  );
  lines.push(
    `    CustomLog \${APACHE_LOG_DIR}/${
      form.serverName.trim() || "site"
    }-access.log combined`,
  );
  lines.push("</VirtualHost>");
  return lines.join("\n") + "\n";
}

function renderCaddySite(form: CaddyForm): string {
  const lines: string[] = [];
  const addr = form.address.trim() || "example.com";
  lines.push(`${addr} {`);
  if (form.mode === "reverse-proxy") {
    lines.push(`    reverse_proxy ${form.upstream.trim() || "127.0.0.1:8080"}`);
  } else {
    lines.push(`    root * ${form.root.trim() || "/var/www/html"}`);
    lines.push("    file_server");
  }
  if (form.encode) {
    lines.push("    encode gzip zstd");
  }
  lines.push("    log {");
  lines.push("        output file /var/log/caddy/access.log");
  lines.push("    }");
  lines.push("}");
  return lines.join("\n") + "\n";
}

function sanitizeLeaf(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
