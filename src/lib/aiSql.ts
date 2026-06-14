// One-shot "describe → SQL" generation. Reuses the configured AI provider
// + the streaming `ai_chat_send` command, but accumulates the response into
// a single string and resolves once — no chat UI, no persisted history.
// The model is told to emit ONLY a single SQL statement in a fenced block,
// which we strip back out for the editor.

import { listen } from "@tauri-apps/api/event";
import * as ai from "./ai";

export type SqlSchemaContext = {
  /** Human dialect name handed to the model: "MySQL" / "PostgreSQL" / … */
  dialect: string;
  database?: string;
  /** All table names in the active database — gives the model the full
   *  catalog to pick from without paying the token cost of every column. */
  tables: string[];
  /** Full column list of the currently-selected table, when one is open. */
  currentTable?: { name: string; columns: { name: string; type: string }[] };
};

function buildPrompt(schema: SqlSchemaContext, description: string): string {
  const lines: string[] = [];
  lines.push(
    `You are an expert ${schema.dialect} engineer. Produce ONE ${schema.dialect} SQL statement that satisfies the request.`,
  );
  lines.push(
    "Output rules: reply with ONLY the SQL inside a single ```sql code fence. No prose, no comments, and do not call any tools or run anything.",
  );
  if (schema.database) lines.push(`Database: ${schema.database}`);
  if (schema.tables.length > 0) {
    lines.push(`Tables in this database: ${schema.tables.join(", ")}`);
  }
  if (schema.currentTable && schema.currentTable.columns.length > 0) {
    const cols = schema.currentTable.columns
      .map((c) => `${c.name} ${c.type}`)
      .join(", ");
    lines.push(`Columns of the open table \`${schema.currentTable.name}\`: ${cols}`);
  }
  lines.push(`Request: ${description}`);
  return lines.join("\n");
}

/** Pull the SQL back out of the model's reply. Prefers a ```sql fence,
 *  falls back to the first generic fence, then the raw text. */
function extractSql(text: string): string {
  const sqlFence = text.match(/```sql\s*([\s\S]*?)```/i);
  if (sqlFence) return sqlFence[1].trim();
  const anyFence = text.match(/```\s*([\s\S]*?)```/);
  if (anyFence) return anyFence[1].trim();
  return text.trim();
}

export async function generateSql(opts: {
  provider: ai.AiProviderSettings;
  schema: SqlSchemaContext;
  description: string;
  /** Abort budget in ms. */
  timeoutMs?: number;
}): Promise<string> {
  const conversationId = `ai-sql:${crypto.randomUUID()}`;
  const userText = buildPrompt(opts.schema, opts.description);
  let acc = "";
  let assistant = "";

  return await new Promise<string>((resolve, reject) => {
    let unlisten: (() => void) | null = null;
    let settled = false;
    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      if (unlisten) unlisten();
      fn();
    };
    const timer = window.setTimeout(
      () => finish(() => reject(new Error("AI request timed out"))),
      opts.timeoutMs ?? 60_000,
    );

    void listen<ai.AiChatEvent>(ai.AI_CHAT_EVENT, (event) => {
      const ev = event.payload;
      if (ev.conversationId !== conversationId) return;
      if (ev.kind === "delta") {
        if (ev.text) acc += ev.text;
      } else if (ev.kind === "assistant") {
        if (ev.text) assistant = ev.text;
      } else if (ev.kind === "done") {
        finish(() => resolve(extractSql(acc || assistant)));
      } else if (ev.kind === "failed") {
        finish(() => reject(new Error(ev.message ?? "AI request failed")));
      }
    }).then((un) => {
      if (settled) {
        un();
        return;
      }
      unlisten = un;
      const req: ai.AiChatRequest = {
        conversationId,
        provider: opts.provider,
        userText,
        attachments: [],
        redact: false,
        askReadOnly: true,
        persistHistory: false,
        ssh: null,
      };
      ai.aiChatSend(req).catch((e) =>
        finish(() => reject(e instanceof Error ? e : new Error(String(e)))),
      );
    });
  });
}
