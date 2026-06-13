/** SSH addressing + auth the preview viewers thread into their
 *  Tauri commands and `pierfs://` URLs. Mirrors the SFTP panel's
 *  `sshArgs` shape. */
export type PreviewSshArgs = {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
  sudoPassword?: string | null;
};

/** Common props every viewer subcomponent receives. */
export type ViewerProps = {
  sshArgs: PreviewSshArgs;
  /** Absolute remote path. */
  path: string;
  /** Leaf name (for labels / extension hints). */
  name: string;
  /** Size in bytes, from the directory listing. */
  size: number;
};
