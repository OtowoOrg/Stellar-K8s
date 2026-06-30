const vscode = require('vscode');

function activate(context) {
  context.subscriptions.push(
    vscode.commands.registerCommand('stellar.portForward', () => {
      const terminal = vscode.window.createTerminal('Stellar Port Forward');
      terminal.sendText('kubectl port-forward svc/stellar-operator 8080:8080 -n stellar-system');
      terminal.show();
    }),
    vscode.commands.registerCommand('stellar.streamLogs', () => {
      const terminal = vscode.window.createTerminal('Stellar Logs');
      terminal.sendText('kubectl stellar logs --all -f');
      terminal.show();
    }),
  );
}

function deactivate() {}

module.exports = { activate, deactivate };
