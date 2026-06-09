/**
 * Registers the splitrs LSP server for all Rust files in the current project.
 *
 * IntelliJ's built-in LSP client (2024.2+) calls [fileOpened] for every file
 * opened in the editor; this provider starts [SplitrsLspServerDescriptor] for
 * any *.rs file, launching `splitrs-lsp` over stdio exactly once per project.
 *
 * Prerequisites: `splitrs-lsp` must be on $PATH (`cargo install splitrs`).
 */
package com.cooljapan.splitrs

import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider
import com.intellij.platform.lsp.api.LspServerSupportProvider.LspServerStarter

class SplitrsLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(
        project: Project,
        file: VirtualFile,
        serverStarter: LspServerStarter,
    ) {
        if (file.extension == "rs") {
            serverStarter.ensureLspServerStarted(SplitrsLspServerDescriptor(project))
        }
    }
}
