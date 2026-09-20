import { useEffect, useRef } from "react";
import type { HostToWebview, WebviewToHost } from "../protocol";

type Listener = (msg: HostToWebview) => void;

declare const acquireVsCodeApi: () => {
  postMessage(msg: WebviewToHost): void;
  getState(): unknown;
  setState(state: unknown): void;
};

const vscodeApi = acquireVsCodeApi();

export function postToHost(msg: WebviewToHost): void {
  vscodeApi.postMessage(msg);
}

export function useHostMessage(listener: Listener): void {
  const ref = useRef(listener);
  useEffect(() => {
    ref.current = listener;
  }, [listener]);
  useEffect(() => {
    const handler = (e: MessageEvent) => {
      const msg = e.data as HostToWebview;
      if (msg && typeof msg === "object" && "kind" in msg) ref.current(msg);
    };
    window.addEventListener("message", handler);
    postToHost({ kind: "ready" });
    return () => window.removeEventListener("message", handler);
  }, []);
}
