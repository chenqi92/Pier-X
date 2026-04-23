export const DT_LOCAL_FILE = "application/x-pier-localfile";
export const DT_SFTP_FILE = "application/x-pier-sftpfile";

const DT_TEXT = "text/plain";
const TEXT_PREFIX = "pier-x-drag:";

export type LocalDragPayload = { path: string; name: string; isDir?: boolean };

export type SftpDragPayload = {
  path: string;
  name: string;
  isDir: boolean;
  size: number;
  host: string;
  port: number;
  user: string;
  authMode: string;
  sourceTabId?: string;
};

type DragPayloadKind = "local-file" | "sftp-file";

type DragPayloadByKind = {
  "local-file": LocalDragPayload | LocalDragPayload[];
  "sftp-file": SftpDragPayload;
};

type DragEnvelope<K extends DragPayloadKind> = {
  kind: K;
  payload: DragPayloadByKind[K];
};

function parseJson<T>(raw: string): T | null {
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

export function hasDragPayload(dataTransfer: DataTransfer, customType: string): boolean {
  const types = Array.from(dataTransfer.types);
  return types.includes(customType) || (types.includes(DT_TEXT) && !types.includes("Files"));
}

export function writeDragPayload<K extends DragPayloadKind>(
  dataTransfer: DataTransfer,
  customType: string,
  kind: K,
  payload: DragPayloadByKind[K],
): void {
  dataTransfer.setData(customType, JSON.stringify(payload));
  dataTransfer.setData(DT_TEXT, `${TEXT_PREFIX}${JSON.stringify({ kind, payload })}`);
}

export function readDragPayload<K extends DragPayloadKind>(
  dataTransfer: DataTransfer,
  customType: string,
  kind: K,
): DragPayloadByKind[K] | null {
  const customRaw = dataTransfer.getData(customType);
  if (customRaw) {
    return parseJson<DragPayloadByKind[K]>(customRaw);
  }

  const textRaw = dataTransfer.getData(DT_TEXT);
  if (!textRaw.startsWith(TEXT_PREFIX)) return null;

  const envelope = parseJson<DragEnvelope<K>>(textRaw.slice(TEXT_PREFIX.length));
  if (!envelope || envelope.kind !== kind) return null;
  return envelope.payload;
}
