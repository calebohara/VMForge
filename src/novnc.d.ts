// Minimal type shim for noVNC (ships no TypeScript types). The package's
// single export is the RFB class (package.json "exports": "./core/rfb.js").
declare module "@novnc/novnc" {
  export interface RFBOptions {
    shared?: boolean;
    credentials?: { username?: string; password?: string; target?: string };
    wsProtocols?: string[];
  }
  export default class RFB {
    constructor(target: HTMLElement, url: string, options?: RFBOptions);
    scaleViewport: boolean;
    resizeSession: boolean;
    background: string;
    viewOnly: boolean;
    disconnect(): void;
    focus(): void;
    addEventListener(type: string, listener: (e: CustomEvent) => void): void;
    removeEventListener(type: string, listener: (e: CustomEvent) => void): void;
  }
}
