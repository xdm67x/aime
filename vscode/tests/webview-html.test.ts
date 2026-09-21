import { describe, expect, it } from "vitest";
import { buildWebviewHtml } from "../src/webview-html";

const opts = {
  cspSource: "https://test.vscode-cdn.net",
  scriptUri: "https://test.vscode-cdn.net/index.js",
  styleUri: "https://test.vscode-cdn.net/index.css",
  nonce: "abc123",
};

describe("webview html", () => {
  it("loads the vite bundle entry, not a generated index.html", () => {
    const html = buildWebviewHtml(opts);
    expect(html).toContain('src="https://test.vscode-cdn.net/index.js"');
    expect(html).not.toContain("index.html");
  });

  it("links the stylesheet", () => {
    const html = buildWebviewHtml(opts);
    expect(html).toContain(
      '<link rel="stylesheet" href="https://test.vscode-cdn.net/index.css" />',
    );
  });

  it("marks the script tag with the CSP nonce", () => {
    const html = buildWebviewHtml(opts);
    expect(html).toContain('nonce="abc123"');
    expect(html).toContain("script-src https://test.vscode-cdn.net 'nonce-abc123'");
    expect(html).not.toContain("strict-dynamic");
  });

  it("renders into a #root container", () => {
    const html = buildWebviewHtml(opts);
    expect(html).toContain('<div id="root"></div>');
  });
});
