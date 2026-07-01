declare module "@novnc/novnc" {
  export type RFBCredentials = {
    username?: string;
    password?: string;
    target?: string;
  };

  export type RFBOptions = {
    credentials?: RFBCredentials;
    shared?: boolean;
    repeaterID?: string;
    wsProtocols?: string[];
  };

  export type RFBClipboardEvent = CustomEvent<{ text: string }>;
  export type RFBDisconnectEvent = CustomEvent<{ clean: boolean }>;
  export type RFBSecurityFailureEvent = CustomEvent<{ status: number; reason: string }>;

  export interface RFBEventMap {
    clipboard: RFBClipboardEvent;
    connect: Event;
    credentialsrequired: Event;
    disconnect: RFBDisconnectEvent;
    securityfailure: RFBSecurityFailureEvent;
  }

  export default class RFB extends EventTarget {
    constructor(target: HTMLElement, url: string, options?: RFBOptions);

    viewOnly: boolean;
    focusOnClick: boolean;
    clipViewport: boolean;
    scaleViewport: boolean;
    resizeSession: boolean;
    qualityLevel: number;
    compressionLevel: number;
    showDotCursor: boolean;
    background: string;

    addEventListener<K extends keyof RFBEventMap>(
      type: K,
      listener: (event: RFBEventMap[K]) => void,
      options?: boolean | AddEventListenerOptions,
    ): void;
    removeEventListener<K extends keyof RFBEventMap>(
      type: K,
      listener: (event: RFBEventMap[K]) => void,
      options?: boolean | EventListenerOptions,
    ): void;

    blur(): void;
    clipboardPasteFrom(text: string): void;
    disconnect(): void;
    focus(): void;
    sendCredentials(credentials: RFBCredentials): void;
  }
}
