/**
 * Describes how to launch `splitrs-lsp` for a project.
 *
 * The server binary is resolved from the `splitrs.serverPath` system property
 * (set via JVM args, e.g. -Dsplitrs.serverPath=/usr/local/bin/splitrs-lsp),
 * falling back to `splitrs-lsp` on $PATH.
 *
 * Coexistence: Because IntelliJ's LSP client supports multiple servers per
 * file type, splitrs-lsp runs alongside rust-analyzer without conflict.
 */
package com.cooljapan.splitrs

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor

private const val DEFAULT_SERVER_PATH = "splitrs-lsp"
private const val SERVER_PATH_PROPERTY = "splitrs.serverPath"

class SplitrsLspServerDescriptor(project: Project) :
    ProjectWideLspServerDescriptor(project, "splitrs") {

    override fun isSupportedFile(file: VirtualFile): Boolean =
        file.extension == "rs"

    override fun createCommandLine(): GeneralCommandLine {
        val serverPath = System.getProperty(SERVER_PATH_PROPERTY, DEFAULT_SERVER_PATH)
        return GeneralCommandLine(serverPath)
            .withWorkDirectory(project.basePath)
    }
}
