//! Arbitrage de l'unique écran de l'overlay entre les guildes.
//!
//! L'overlay n'a qu'un écran, toutes guildes confondues : une seule session
//! WebSocket par guilde, mais un seul média affiché à la fois. Ce module porte
//! la décision « qui occupe l'écran, qui attend en file locale, à qui router
//! une trame de télémétrie ». Toutes les décisions se prennent sous un même
//! verrou ([`ScreenArbiter`]), ce qui interdit l'affichage simultané de deux
//! médias par deux sessions qui auraient lu « écran libre » en même temps.
//!
//! Trois règles portent la correction du bug multi-guilde :
//!
//! - un `play` n'est affiché que si l'écran est libre ou déjà à sa guilde ;
//!   sinon il est mis en file (dédoublonné par `job_id`) ;
//! - un rapport de fin de lecture ne libère l'écran que si le job rapporté y
//!   est **encore** : un rapport tardif (le renderer émet un `playback.state`
//!   final, un `stop` serveur a déjà fait passer au média suivant) n'efface
//!   jamais le média qui a pris sa place ;
//! - la guilde propriétaire d'un job est mémorisée pour les derniers jobs
//!   affichés, afin de router `playback.ended` / `playback.state` vers la
//!   guilde qui possède le job et non vers celle du média suivant.

use std::collections::VecDeque;

use serde_json::Value;

/// `play` reçu d'une guilde pendant qu'une autre occupe l'écran.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedPlay {
    pub guild_id: String,
    pub job_id: String,
    pub payload: Value,
}

/// Job identifié avec la guilde qui le possède.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedJob {
    pub guild_id: String,
    pub job_id: String,
}

/// Ce qu'il faut faire d'un `play` reçu.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayPresentation {
    /// Afficher ce payload (l'écran était libre ou déjà à cette guilde).
    Display(Value),
    /// Garder le `play` en file locale : une autre guilde occupe l'écran.
    Queued,
}

/// Effet d'un `stop` serveur (ou d'une fin de session) sur l'écran et la file.
#[derive(Debug, Clone, PartialEq)]
pub struct StopDecision {
    /// Nombre de `play` retirés de la file locale.
    pub dropped: usize,
    /// L'écran a été libéré : il faut délivrer `stop` au renderer.
    pub display_cleared: bool,
    /// `play` suivant à afficher, s'il y en a un.
    pub next: Option<QueuedPlay>,
}

/// Effet d'un rapport de fin de lecture émis par le renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportDecision {
    /// Job terminé et guilde à qui envoyer `playback.ended`, s'il est connu.
    pub ended_for: Option<OwnedJob>,
    /// Le média affiché était bien celui du rapport : l'écran est libéré.
    pub display_cleared: bool,
    /// `play` suivant à afficher, s'il y en a un.
    pub next: Option<QueuedPlay>,
}

/// Nombre de jobs affichés dont on garde la trace pour router un rapport
/// tardif vers la guilde qui les possédait.
const RECENT_LIMIT: usize = 16;

/// Arbitre de l'écran unique : propriétaire courant, file locale et mémoire
/// des derniers jobs affichés.
#[derive(Debug, Default)]
pub struct ScreenArbiter {
    screen: Option<OwnedJob>,
    queue: VecDeque<QueuedPlay>,
    recent: VecDeque<OwnedJob>,
}

impl ScreenArbiter {
    /// Job à l'écran, toutes guildes confondues.
    pub fn screen(&self) -> Option<&OwnedJob> {
        self.screen.as_ref()
    }

    /// Guilde propriétaire du job à l'écran.
    pub fn screen_guild(&self) -> Option<&str> {
        self.screen.as_ref().map(|owner| owner.guild_id.as_str())
    }

    /// Guilde propriétaire d'un job : l'écran, les derniers affichés, puis la
    /// file locale (une trame peut concerner un `play` qui attend son tour).
    pub fn owner_of(&self, job_id: &str) -> Option<&str> {
        if let Some(owner) = self.known(job_id) {
            return Some(owner.guild_id.as_str());
        }
        self.queue
            .iter()
            .find(|queued| queued.job_id == job_id)
            .map(|queued| queued.guild_id.as_str())
    }

    /// `(job_id, durée en ms)` des `play` d'une guilde en file locale, dans
    /// l'ordre d'attente.
    pub fn queued_jobs_for(&self, guild_id: &str) -> Vec<(String, u64)> {
        self.queue
            .iter()
            .filter(|queued| queued.guild_id == guild_id)
            .map(|queued| (queued.job_id.clone(), queued_duration_ms(&queued.payload)))
            .collect()
    }

    /// Décide du sort d'un `play` : affichage immédiat ou mise en attente.
    ///
    /// `showable` dit si l'overlay peut montrer quelque chose (activé et
    /// appairé) : sinon le `play` attend, pour ne pas devenir un propriétaire
    /// fantôme que le renderer n'a jamais reçu.
    pub fn present(
        &mut self,
        guild_id: &str,
        job_id: &str,
        payload: Value,
        showable: bool,
    ) -> PlayPresentation {
        let free_for_this_guild = self
            .screen
            .as_ref()
            .is_none_or(|owner| owner.guild_id == guild_id);
        if !showable || !free_for_this_guild {
            self.enqueue(QueuedPlay {
                guild_id: guild_id.to_string(),
                job_id: job_id.to_string(),
                payload,
            });
            return PlayPresentation::Queued;
        }
        // Réaffichage du même job (reconnexion : le serveur renvoie `current_job`)
        // ou remplacement par un autre `play` de la même guilde : il ne doit
        // rester qu'une entrée par job, et le job remplacé garde sa guilde
        // pour le routage de son rapport de fin.
        self.queue.retain(|queued| queued.job_id != job_id);
        if let Some(previous) = self.screen.take() {
            if previous.job_id != job_id {
                self.remember(previous);
            }
        }
        self.screen = Some(OwnedJob {
            guild_id: guild_id.to_string(),
            job_id: job_id.to_string(),
        });
        PlayPresentation::Display(payload)
    }

    /// Affiche le prochain `play` en attente si l'écran est libre.
    pub fn advance(&mut self, showable: bool) -> Option<QueuedPlay> {
        if self.screen.is_some() || !showable {
            return None;
        }
        let next = self.queue.pop_front()?;
        self.screen = Some(OwnedJob {
            guild_id: next.guild_id.clone(),
            job_id: next.job_id.clone(),
        });
        Some(next)
    }

    /// `stop` reçu du serveur pour une guilde, ou fin de sa session.
    ///
    /// La file de cette guilde est toujours purgée (le job ciblé, ou tous ses
    /// jobs). L'écran n'est libéré que si cette guilde y est encore, pour ce
    /// job précis ou sans job ciblé : un `stop` qui arrive après que le média
    /// suivant a pris l'écran ne doit pas l'effacer.
    pub fn server_stop(
        &mut self,
        guild_id: &str,
        job_id: Option<&str>,
        showable: bool,
    ) -> StopDecision {
        let dropped = self.drop_queued(guild_id, job_id);
        let owns_screen = self.screen.as_ref().is_some_and(|owner| {
            owner.guild_id == guild_id && job_id.is_none_or(|job_id| owner.job_id == job_id)
        });
        if !owns_screen {
            return StopDecision {
                dropped,
                display_cleared: false,
                next: None,
            };
        }
        let screen = self.screen.take();
        if let Some(previous) = &screen {
            self.remember(previous.clone());
        }
        StopDecision {
            dropped,
            display_cleared: true,
            next: self.advance(showable),
        }
    }

    /// Fin de session d'une guilde : purge sa file et libère l'écran si elle
    /// l'occupait.
    pub fn release_guild(&mut self, guild_id: &str, showable: bool) -> StopDecision {
        self.server_stop(guild_id, None, showable)
    }

    /// Rapport de fin de lecture du renderer (`job_id` absent = média affiché).
    ///
    /// Le job affiché est toujours libéré et son propriétaire retourné pour le
    /// routage. Un job connu mais qui n'est plus à l'écran (rapport tardif)
    /// est routé vers sa guilde sans toucher à l'écran. Un job inconnu est
    /// ignoré : rien ne prouve qu'il concernait l'écran.
    pub fn report_stop(&mut self, job_id: Option<&str>, showable: bool) -> ReportDecision {
        let Some(ended_for) = self.known_owned(job_id) else {
            return ReportDecision {
                ended_for: None,
                display_cleared: false,
                next: None,
            };
        };
        if self.screen.as_ref() != Some(&ended_for) {
            return ReportDecision {
                ended_for: Some(ended_for),
                display_cleared: false,
                next: None,
            };
        }
        self.screen = None;
        self.remember(ended_for.clone());
        ReportDecision {
            ended_for: Some(ended_for),
            display_cleared: true,
            next: self.advance(showable),
        }
    }

    /// Met un `play` en file sans doublon : un job déjà en attente garde sa
    /// place (un `welcome` de reconnexion ne réordonne pas la file).
    fn enqueue(&mut self, queued: QueuedPlay) {
        if let Some(existing) = self
            .queue
            .iter_mut()
            .find(|existing| existing.job_id == queued.job_id)
        {
            existing.payload = queued.payload;
            return;
        }
        self.queue.push_back(queued);
    }

    /// Retire les `play` en attente d'une guilde (un job précis, ou tous).
    fn drop_queued(&mut self, guild_id: &str, job_id: Option<&str>) -> usize {
        let before = self.queue.len();
        self.queue.retain(|queued| {
            queued.guild_id != guild_id || job_id.is_some_and(|job_id| queued.job_id != job_id)
        });
        before - self.queue.len()
    }

    /// Job et guilde correspondant à `job_id` (`None` = job affiché).
    fn known_owned(&self, job_id: Option<&str>) -> Option<OwnedJob> {
        match job_id {
            Some(job_id) => self.known(job_id).cloned(),
            None => self.screen.clone(),
        }
    }

    fn known(&self, job_id: &str) -> Option<&OwnedJob> {
        self.screen
            .as_ref()
            .filter(|owner| owner.job_id == job_id)
            .or_else(|| self.recent.iter().find(|owner| owner.job_id == job_id))
    }

    fn remember(&mut self, job: OwnedJob) {
        self.recent.retain(|known| known.job_id != job.job_id);
        self.recent.push_back(job);
        while self.recent.len() > RECENT_LIMIT {
            self.recent.pop_front();
        }
    }
}

/// Durée de lecture d'un `play` en file locale : `durationSec` en ms, 5 s par
/// défaut si absent/invalide, plafonnée par le protocole.
fn queued_duration_ms(payload: &Value) -> u64 {
    payload["durationSec"]
        .as_u64()
        .map(|duration_sec| duration_sec.saturating_mul(1_000))
        .unwrap_or(5_000)
        .min(lc_protocol::MAX_PLAYBACK_REMAINING_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUILD_A: &str = "111";
    const GUILD_B: &str = "222";
    const GUILD_C: &str = "333";

    fn play(guild_id: &str, job_id: &str) -> QueuedPlay {
        QueuedPlay {
            guild_id: guild_id.to_string(),
            job_id: job_id.to_string(),
            payload: payload(job_id),
        }
    }

    fn payload(job_id: &str) -> Value {
        serde_json::json!({ "jobId": job_id, "durationSec": 4 })
    }

    /// Présente un `play` et exige qu'il prenne l'écran.
    fn present(arbiter: &mut ScreenArbiter, guild_id: &str, job_id: &str) {
        let decision = arbiter.present(guild_id, job_id, payload(job_id), true);
        assert_eq!(
            decision,
            PlayPresentation::Display(payload(job_id)),
            "le play {job_id} aurait dû prendre l'écran"
        );
        assert_eq!(arbiter.screen_guild(), Some(guild_id));
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some(job_id)
        );
    }

    /// Présente un `play` et exige qu'il reste en file.
    fn queue(arbiter: &mut ScreenArbiter, guild_id: &str, job_id: &str) {
        let decision = arbiter.present(guild_id, job_id, payload(job_id), true);
        assert_eq!(
            decision,
            PlayPresentation::Queued,
            "le play {job_id} aurait dû attendre son tour"
        );
    }

    fn queue_len(arbiter: &ScreenArbiter) -> usize {
        arbiter.queue.len()
    }

    fn assert_quiescent(arbiter: &ScreenArbiter, context: &str) {
        // Aucun média ne doit rester en file quand plus rien n'est à l'écran :
        // c'est le symptôme rapporté (des médias « en file » alors que rien ne
        // joue) que l'arbitre rend impossible.
        assert!(
            arbiter.screen.is_some() || arbiter.queue.is_empty(),
            "{context} : file non vide sans média à l'écran"
        );
        // Un job ne peut pas être à la fois à l'écran et en file, et la file
        // ne contient jamais deux fois le même job.
        let mut seen = std::collections::HashSet::new();
        for queued in &arbiter.queue {
            assert!(
                seen.insert(queued.job_id.clone()),
                "{context} : doublon en file"
            );
            assert_ne!(
                arbiter.screen.as_ref().map(|owner| owner.job_id.as_str()),
                Some(queued.job_id.as_str()),
                "{context} : job à l'écran également en file"
            );
        }
    }

    // --- Affichage unique ---------------------------------------------------

    #[test]
    fn deux_plays_simultanes_n_affichent_qu_un_media() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        assert_eq!(arbiter.screen_guild(), Some(GUILD_A));
        assert_eq!(
            arbiter.queued_jobs_for(GUILD_B),
            vec![("b1".to_string(), 4_000)]
        );
        assert_quiescent(&arbiter, "deux plays simultanés");
    }

    #[test]
    fn deux_sessions_concurrentes_ne_prennent_pas_l_ecran_ensemble() {
        // L'ancien code lisait « écran libre » puis écrivait le propriétaire
        // sans verrou commun : deux sessions pouvaient afficher deux médias.
        use std::sync::{Arc, Barrier, Mutex};

        for _round in 0..50 {
            let arbiter = Arc::new(Mutex::new(ScreenArbiter::default()));
            let barrier = Arc::new(Barrier::new(2));
            let mut handles = Vec::new();
            for guild_id in [GUILD_A, GUILD_B] {
                let arbiter = Arc::clone(&arbiter);
                let barrier = Arc::clone(&barrier);
                handles.push(std::thread::spawn(move || {
                    barrier.wait();
                    let mut guard = arbiter.lock().expect("verrou");
                    guard.present(guild_id, "job", payload("job"), true)
                }));
            }
            let decisions = handles
                .into_iter()
                .map(|handle| handle.join().expect("thread"))
                .collect::<Vec<_>>();
            let displayed = decisions
                .iter()
                .filter(|decision| matches!(decision, PlayPresentation::Display(_)))
                .count();
            assert_eq!(displayed, 1, "un seul play doit prendre l'écran");
        }
    }

    #[test]
    fn un_play_de_la_meme_guilde_remplace_l_ecran() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        present(&mut arbiter, GUILD_A, "a2");
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("a2")
        );
        assert_eq!(queue_len(&arbiter), 0);
    }

    #[test]
    fn un_play_reeemis_apres_reconnexion_ne_double_pas_la_file() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        queue(&mut arbiter, GUILD_B, "b1");
        assert_eq!(arbiter.queued_jobs_for(GUILD_B).len(), 1);
        assert!(
            matches!(
                arbiter.report_stop(Some("a1"), true).next,
                Some(QueuedPlay { .. })
            ),
            "le play réémis doit être le suivant"
        );
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("b1")
        );
    }

    #[test]
    fn la_file_est_servie_dans_l_ordre_d_arrivee() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        queue(&mut arbiter, GUILD_C, "c1");

        let decision = arbiter.report_stop(Some("a1"), true);
        assert_eq!(decision.next.map(|next| next.job_id).as_deref(), Some("b1"));
        let decision = arbiter.report_stop(Some("b1"), true);
        assert_eq!(decision.next.map(|next| next.job_id).as_deref(), Some("c1"));
        let decision = arbiter.report_stop(Some("c1"), true);
        assert!(decision.next.is_none());
        assert_quiescent(&arbiter, "file servie");
        assert!(arbiter.screen().is_none());
    }

    #[test]
    fn un_play_reeemis_en_file_garde_sa_place() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        queue(&mut arbiter, GUILD_C, "c1");
        queue(&mut arbiter, GUILD_B, "b1");
        let decision = arbiter.report_stop(Some("a1"), true);
        assert_eq!(decision.next.map(|next| next.job_id).as_deref(), Some("b1"));
        let decision = arbiter.report_stop(Some("b1"), true);
        assert_eq!(decision.next.map(|next| next.job_id).as_deref(), Some("c1"));
    }

    #[test]
    fn un_play_recu_quand_l_overlay_est_masque_reste_en_file() {
        let mut arbiter = ScreenArbiter::default();
        assert_eq!(
            arbiter.present(GUILD_A, "a1", payload("a1"), false),
            PlayPresentation::Queued
        );
        assert!(arbiter.screen().is_none());
        assert_eq!(queue_len(&arbiter), 1);

        // Rendre l'écran le réaffiche sans jamais recevoir `play` deux fois.
        present(&mut arbiter, GUILD_A, "a1");
        assert_eq!(queue_len(&arbiter), 0);
        assert_quiescent(&arbiter, "overlay réactivé");
    }

    // --- Fin de lecture -----------------------------------------------------

    #[test]
    fn un_rapport_sans_job_termine_le_media_affiche() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        let decision = arbiter.report_stop(None, true);
        assert_eq!(decision.ended_for, Some(owned(GUILD_A, "a1")));
        assert!(decision.display_cleared);
        assert!(arbiter.screen().is_none());
    }

    #[test]
    fn un_rapport_tardif_n_efface_pas_le_media_suivant() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");

        // Le serveur a stoppé a1 et b1 a pris l'écran...
        let decision = arbiter.server_stop(GUILD_A, Some("a1"), true);
        assert!(decision.display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("b1")
        );

        // ... puis le renderer rapporte (en retard) la fin de a1.
        let decision = arbiter.report_stop(Some("a1"), true);
        assert_eq!(decision.ended_for, Some(owned(GUILD_A, "a1")));
        assert!(!decision.display_cleared, "b1 ne doit pas être effacé");
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("b1")
        );
    }

    #[test]
    fn un_rapport_du_meme_job_deux_fois_ne_libere_pas_deux_medias() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        assert!(arbiter.report_stop(Some("a1"), true).display_cleared);
        let decision = arbiter.report_stop(Some("a1"), true);
        assert!(decision.ended_for.is_some());
        assert!(!decision.display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("b1")
        );
    }

    #[test]
    fn un_rapport_d_un_job_inconnu_est_ignore() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        let decision = arbiter.report_stop(Some("inconnu"), true);
        assert_eq!(decision.ended_for, None);
        assert!(!decision.display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("a1")
        );
    }

    #[test]
    fn le_rapport_d_un_media_qui_vient_d_etre_remplace_ne_libere_pas_l_ecran() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        present(&mut arbiter, GUILD_A, "a2");
        let decision = arbiter.report_stop(Some("a1"), true);
        assert_eq!(decision.ended_for, Some(owned(GUILD_A, "a1")));
        assert!(!decision.display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("a2")
        );
    }

    #[test]
    fn la_fin_du_media_route_vers_sa_guilde_meme_apres_remplacement() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        present(&mut arbiter, GUILD_A, "a2");
        queue(&mut arbiter, GUILD_B, "b1");
        // Trames tardives du renderer pour a1 : la guilde A est connue.
        assert_eq!(arbiter.owner_of("a1"), Some(GUILD_A));
        assert_eq!(arbiter.owner_of("a2"), Some(GUILD_A));
        assert_eq!(arbiter.owner_of("b1"), Some(GUILD_B));
        assert_eq!(arbiter.owner_of("inconnu"), None);
    }

    // --- Stops serveur et fins de session ----------------------------------

    #[test]
    fn un_stop_serveur_de_la_guilde_affichee_libere_l_ecran() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        let decision = arbiter.server_stop(GUILD_A, Some("a1"), true);
        assert!(decision.display_cleared);
        assert_eq!(decision.next.map(|next| next.job_id).as_deref(), Some("b1"));
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("b1")
        );
    }

    #[test]
    fn un_stop_serveur_sans_job_libere_le_media_de_sa_guilde() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        let decision = arbiter.server_stop(GUILD_A, None, true);
        assert!(decision.display_cleared);
        assert!(arbiter.screen().is_none());
    }

    #[test]
    fn un_stop_serveur_d_une_autre_guilde_ne_libere_pas_l_ecran() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        let decision = arbiter.server_stop(GUILD_B, Some("b1"), true);
        assert_eq!(decision.dropped, 1);
        assert!(!decision.display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("a1")
        );
        assert!(arbiter.queued_jobs_for(GUILD_B).is_empty());
    }

    #[test]
    fn un_stop_serveur_pour_un_job_deja_remplace_ne_libere_pas_l_ecran() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        present(&mut arbiter, GUILD_A, "a2");
        let decision = arbiter.server_stop(GUILD_A, Some("a1"), true);
        assert!(!decision.display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("a2")
        );
    }

    #[test]
    fn un_stop_cible_ne_retire_que_le_job_vise_de_la_file() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        queue(&mut arbiter, GUILD_B, "b2");
        assert_eq!(arbiter.server_stop(GUILD_B, Some("b1"), true).dropped, 1);
        assert_eq!(
            arbiter
                .queued_jobs_for(GUILD_B)
                .into_iter()
                .map(|(job_id, _)| job_id)
                .collect::<Vec<_>>(),
            vec!["b2".to_string()]
        );
        assert_eq!(arbiter.server_stop(GUILD_B, None, true).dropped, 1);
        assert!(arbiter.queued_jobs_for(GUILD_B).is_empty());
    }

    #[test]
    fn la_fin_d_une_session_purge_sa_file_et_passe_au_suivant() {
        let mut arbiter = ScreenArbiter::default();
        // C occupe l'écran quand la session de B se termine : les `play` de B
        // en attente sont purgés, C garde l'écran.
        present(&mut arbiter, GUILD_C, "c1");
        queue(&mut arbiter, GUILD_B, "b1");
        queue(&mut arbiter, GUILD_B, "b2");
        let decision = arbiter.release_guild(GUILD_B, true);
        assert_eq!(decision.dropped, 2);
        assert!(!decision.display_cleared);
        assert!(decision.next.is_none());
        assert_eq!(arbiter.screen_guild(), Some(GUILD_C));

        // Puis C se termine : plus rien en file, l'écran est libre.
        let decision = arbiter.release_guild(GUILD_C, true);
        assert!(decision.display_cleared);
        assert!(decision.next.is_none());
        assert!(arbiter.screen().is_none());
        assert_quiescent(&arbiter, "après la fin des deux sessions");
    }

    #[test]
    fn la_fin_du_media_affiche_passe_au_play_suivant_de_la_file() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        let decision = arbiter.release_guild(GUILD_A, true);
        assert!(decision.display_cleared);
        assert_eq!(decision.next.map(|next| next.job_id).as_deref(), Some("b1"));
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("b1")
        );
    }

    #[test]
    fn une_guilde_absente_de_l_ecran_peut_se_deconnecter_sans_l_affecter() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        let decision = arbiter.release_guild(GUILD_B, true);
        assert_eq!(decision.dropped, 1);
        assert!(!decision.display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("a1")
        );
    }

    // --- Invariants et séquences aléatoires --------------------------------

    #[test]
    fn sequencer_deux_guildes_ne_laisse_jamais_de_media_fantome() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        queue(&mut arbiter, GUILD_B, "b1");
        // a1 se termine côté renderer : b1 s'affiche.
        assert!(arbiter.report_stop(Some("a1"), true).display_cleared);
        assert_eq!(
            arbiter.screen().map(|owner| owner.job_id.as_str()),
            Some("b1")
        );
        // b1 se termine : plus rien à l'écran ni en file.
        assert!(arbiter.report_stop(Some("b1"), true).display_cleared);
        assert!(arbiter.screen().is_none());
        assert_quiescent(&arbiter, "fin de b1");
        // Un nouveau média peut prendre l'écran immédiatement.
        present(&mut arbiter, GUILD_A, "a2");
    }

    #[test]
    fn sequences_aleatoires_conservent_les_invariants() {
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                self.0 = self
                    .0
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                self.0 >> 33
            }
            fn below(&mut self, bound: usize) -> usize {
                (self.next() % bound as u64) as usize
            }
        }

        let guilds = [GUILD_A, GUILD_B, GUILD_C];
        let mut rng = Rng(0x5EED_1234);
        let mut arbiter = ScreenArbiter::default();
        let mut presented: Vec<String> = Vec::new();
        let mut counter = 0;

        for step in 0..2_000 {
            match rng.below(6) {
                0 | 1 => {
                    counter += 1;
                    let guild_id = guilds[rng.below(guilds.len())];
                    let job_id = format!("job-{counter}");
                    presented.push(job_id.clone());
                    let decision = arbiter.present(guild_id, &job_id, payload(&job_id), true);
                    if matches!(decision, PlayPresentation::Display(_)) {
                        assert_eq!(
                            arbiter.screen_guild(),
                            Some(guild_id),
                            "étape {step} : l'écran doit appartenir à la guilde affichée"
                        );
                    }
                }
                2 => {
                    drop(arbiter.advance(true));
                }
                3 | 4 if presented.is_empty() => {
                    drop(arbiter.advance(true));
                }
                3 => {
                    let guild_id = guilds[rng.below(guilds.len())];
                    let job_id = presented
                        .get(rng.below(presented.len()))
                        .cloned()
                        .unwrap_or_default();
                    let target = (rng.below(2) == 0).then_some(job_id.as_str());
                    drop(arbiter.server_stop(guild_id, target, true));
                }
                4 => {
                    let job_id = presented
                        .get(rng.below(presented.len()))
                        .cloned()
                        .unwrap_or_default();
                    let target = (rng.below(2) == 0).then_some(job_id.as_str());
                    drop(arbiter.report_stop(target, true));
                }
                _ => {
                    let guild_id = guilds[rng.below(guilds.len())];
                    drop(arbiter.release_guild(guild_id, true));
                }
            }
            assert_quiescent(&arbiter, &format!("étape {step}"));
        }
    }

    fn owned(guild_id: &str, job_id: &str) -> OwnedJob {
        OwnedJob {
            guild_id: guild_id.to_string(),
            job_id: job_id.to_string(),
        }
    }

    #[test]
    fn queued_jobs_reports_durations_per_guild() {
        let mut arbiter = ScreenArbiter::default();
        present(&mut arbiter, GUILD_A, "a1");
        arbiter.enqueue(play(GUILD_B, "b1"));
        arbiter.enqueue(QueuedPlay {
            guild_id: GUILD_B.to_string(),
            job_id: "b2".to_string(),
            payload: serde_json::json!({}),
        });
        arbiter.enqueue(QueuedPlay {
            guild_id: GUILD_B.to_string(),
            job_id: "b3".to_string(),
            payload: serde_json::json!({ "durationSec": "sept" }),
        });
        arbiter.enqueue(QueuedPlay {
            guild_id: GUILD_B.to_string(),
            job_id: "b4".to_string(),
            payload: serde_json::json!({ "durationSec": 1_000_000_000 }),
        });
        assert_eq!(
            arbiter.queued_jobs_for(GUILD_B),
            vec![
                ("b1".to_string(), 4_000),
                ("b2".to_string(), 5_000),
                ("b3".to_string(), 5_000),
                ("b4".to_string(), lc_protocol::MAX_PLAYBACK_REMAINING_MS),
            ]
        );
        assert!(arbiter.queued_jobs_for(GUILD_C).is_empty());
    }
}
