package com.cooljapan.splitrs

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Document

/**
 * Validates the structural integrity of plugin.xml without requiring a running
 * IntelliJ instance.  Runs via `./gradlew test` on any JDK 21.
 */
class PluginXmlTest {

    private val pluginXml: Document by lazy {
        val stream = PluginXmlTest::class.java.classLoader
            .getResourceAsStream("META-INF/plugin.xml")
            ?: error("META-INF/plugin.xml not found on classpath")
        DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(stream)
    }

    private fun text(tag: String): String =
        pluginXml.getElementsByTagName(tag).item(0)?.textContent?.trim() ?: ""

    @Test
    fun `plugin id matches expected value`() {
        assertEquals("com.cooljapan.splitrs", text("id"))
    }

    @Test
    fun `plugin depends on platform module`() {
        val deps = pluginXml.getElementsByTagName("depends")
        val depTexts = (0 until deps.length).map { deps.item(it).textContent.trim() }
        assertTrue(depTexts.contains("com.intellij.modules.platform")) {
            "Expected com.intellij.modules.platform in <depends>; got: $depTexts"
        }
    }

    @Test
    fun `lsp server support provider extension is declared`() {
        val extensions = pluginXml.getElementsByTagName("platform.lsp.serverSupportProvider")
        assertTrue(extensions.length > 0) {
            "Expected at least one <platform.lsp.serverSupportProvider> element"
        }
        val impl = extensions.item(0)
            .attributes
            .getNamedItem("implementation")
            ?.nodeValue
        assertEquals("com.cooljapan.splitrs.SplitrsLspServerSupportProvider", impl)
    }

    @Test
    fun `vendor is COOLJAPAN OU`() {
        val vendor = pluginXml.getElementsByTagName("vendor").item(0)
        assertNotNull(vendor)
        assertTrue(vendor.textContent.contains("COOLJAPAN")) {
            "Vendor should mention COOLJAPAN; got: ${vendor.textContent}"
        }
    }
}
