#[cfg(feature = "lsp")]
use splitrs::lsp::Backend;
#[cfg(feature = "lsp")]
use tower_lsp::{LspService, Server};

#[cfg(feature = "lsp")]
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    eprintln!(
        "splitrs-lsp v{} starting on stdio",
        env!("CARGO_PKG_VERSION")
    );
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::build(Backend::new).finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(not(feature = "lsp"))]
fn main() {
    eprintln!("splitrs-lsp requires the 'lsp' feature. Build with --features lsp.");
    std::process::exit(1);
}
