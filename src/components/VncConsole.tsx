import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";

/**
 * Embedded guest console. Connects noVNC (RFB over WebSocket) to the Rust
 * VNC bridge at `ws://127.0.0.1:<wsPort>`.
 */
export function VncConsole({ wsPort }: { wsPort: number }) {
  const ref = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<"connecting" | "connected" | "disconnected">(
    "connecting",
  );

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    setStatus("connecting");
    const rfb = new RFB(el, `ws://127.0.0.1:${wsPort}`, {});
    rfb.scaleViewport = true;
    rfb.background = "#000";

    const onConnect = () => setStatus("connected");
    const onDisconnect = () => setStatus("disconnected");
    rfb.addEventListener("connect", onConnect);
    rfb.addEventListener("disconnect", onDisconnect);

    return () => {
      rfb.removeEventListener("connect", onConnect);
      rfb.removeEventListener("disconnect", onDisconnect);
      try {
        rfb.disconnect();
      } catch {
        /* already gone */
      }
    };
  }, [wsPort]);

  return (
    <div className="relative h-full w-full bg-black">
      <div ref={ref} className="h-full w-full" />
      {status !== "connected" && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-sm text-muted-foreground">
          {status === "connecting" ? "Connecting to console…" : "Console disconnected"}
        </div>
      )}
    </div>
  );
}
