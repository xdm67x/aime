/**
 * Builds the webview document for the Pulse UI. Kept free of any `vscode`
 * import so it can be unit-tested: callers pass webview URIs as strings.
 */

export function buildWebviewHtml(options: {
  cspSource: string;
  scriptUri: string;
  styleUri: string;
  nonce: string;
}): string {
  const { cspSource, scriptUri, styleUri, nonce } = options;
  const csp = [
    "default-src 'none'",
    `img-src ${cspSource} data: blob:`,
    `style-src ${cspSource} 'unsafe-inline'`,
    `script-src ${cspSource} 'nonce-${nonce}'`,
    `font-src ${cspSource}`,
    `connect-src ${cspSource}`,
  ].join("; ");
  return `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta http-equiv="Content-Security-Policy" content="${csp}" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <link rel="stylesheet" href="${styleUri}" />
    <title>Pulse</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" nonce="${nonce}" src="${scriptUri}"></script>
  </body>
</html>`;
}
