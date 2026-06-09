/// splitrs VSCode Extension
///
/// Connects VSCode to the `splitrs-lsp` language server for real-time Rust
/// file-size analysis and one-click refactoring.
///
/// Usage:
///   1. Install splitrs: `cargo install splitrs`
///   2. Install this extension from the VSCode marketplace or via sideloading:
///      cd editors/vscode && npm install && npx vsce package && code --install-extension splitrs-*.vsix
///   3. Open a Rust project. splitrs-lsp activates automatically.
///   4. Use "splitrs: Refactor current file" from the Command Palette (Ctrl+Shift+P)
///      to split an oversized Rust file directly from the editor.
///
/// Coexistence: splitrs-lsp runs alongside rust-analyzer — both can be active
/// simultaneously since the document selector is non-exclusive.

import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
    ExecuteCommandRequest,
} from 'vscode-languageclient/node';

/// Module-level client reference so `deactivate()` can stop the client
/// without requiring it to be passed through closures.
let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    const config = vscode.workspace.getConfiguration('splitrs');

    // Bail out early if the user has disabled the extension.
    const enabled = config.get<boolean>('enable') ?? true;
    if (!enabled) {
        return;
    }

    const serverPath = config.get<string>('serverPath') ?? 'splitrs-lsp';

    const serverOptions: ServerOptions = {
        command: serverPath,
        transport: TransportKind.stdio,
    };

    // The file system watcher forwards `.splitrs.toml` change events to the
    // server so its config-cache invalidation fires. The server itself does NOT
    // self-register a workspace/didChangeWatchedFiles watcher — this client-side
    // registration is the only mechanism that triggers cache invalidation.
    const configWatcher = vscode.workspace.createFileSystemWatcher('**/.splitrs.toml');

    const traceOutputChannel = vscode.window.createOutputChannel('splitrs-lsp Trace');

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'rust' }],
        synchronize: {
            fileEvents: configWatcher,
        },
        outputChannelName: 'splitrs-lsp',
        traceOutputChannel,
        initializationOptions: {
            trace: { server: config.get<string>('trace.server') ?? 'off' },
        },
    };

    client = new LanguageClient(
        'splitrs',
        'splitrs-lsp',
        serverOptions,
        clientOptions,
    );

    // Start the language client and await readiness so the command handler can
    // safely call sendRequest without racing against initialization.
    await client.start();

    // Push the client to subscriptions so it is stopped when the extension
    // is deactivated. LanguageClient implements Disposable in v9.
    context.subscriptions.push(client);

    // Also push the file watcher and trace channel so they are cleaned up.
    context.subscriptions.push(configWatcher);
    context.subscriptions.push(traceOutputChannel);

    // Register the "Refactor current file" command.  The server performs the
    // actual workspace edit via client.apply_edit — the extension only needs
    // to send the execute-command request.
    const commandDisposable = vscode.commands.registerCommand(
        'splitrs.refactorCurrentFile',
        async () => {
            const editor = vscode.window.activeTextEditor;

            if (editor === undefined) {
                await vscode.window.showWarningMessage(
                    'splitrs: No active editor. Open a Rust file first.',
                );
                return;
            }

            if (editor.document.languageId !== 'rust') {
                await vscode.window.showWarningMessage(
                    'splitrs: The active file is not a Rust file.',
                );
                return;
            }

            const uri = editor.document.uri.toString();

            if (client === undefined) {
                await vscode.window.showErrorMessage(
                    'splitrs: Language server is not running.',
                );
                return;
            }

            try {
                await client.sendRequest(ExecuteCommandRequest.type, {
                    command: 'splitrs.split',
                    arguments: [{ uri }],
                });
            } catch (err: unknown) {
                const message =
                    err instanceof Error ? err.message : String(err);
                await vscode.window.showErrorMessage(
                    `splitrs: Failed to execute refactoring — ${message}`,
                );
            }
        },
    );

    context.subscriptions.push(commandDisposable);
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
