//! Runtime tokio pour le réseau et le trousseau.
//!
//! GTK tourne sur sa propre boucle ; les tâches réseau tournent sur ce
//! runtime, et l'interface attend leur résultat avec
//! `glib::spawn_future_local(async { runtime().spawn(...).await })`.

use std::sync::OnceLock;

use tokio::runtime::Runtime;

pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tsubame-reseau")
            .enable_all()
            .build()
            .expect("impossible de démarrer le runtime tokio")
    })
}
