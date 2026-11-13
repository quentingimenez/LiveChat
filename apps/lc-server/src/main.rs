//! Point d'entrée du binaire `lc-server`.
//!
//! Toute la logique vit dans la bibliothèque [`lc_server`] : ce fichier se
//! contente de résoudre la configuration d'environnement, de démarrer
//! [`lc_server::run`] et de convertir les erreurs en code de sortie. Aucun
//! `panic` : une configuration invalide produit un message clair sur `stderr` et
//! le code 1.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use lc_server::Config;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("configuration invalide : {error}");
            return ExitCode::FAILURE;
        }
    };

    match lc_server::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("erreur fatale : {error:#}");
            ExitCode::FAILURE
        }
    }
}
