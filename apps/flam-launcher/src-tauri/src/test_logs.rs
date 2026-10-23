//! Aide aux tests qui capturent les journaux (`tracing`) d'un thread (#400).

use std::sync::Once;

/// Installe, une fois par processus, un abonné global qui accepte tout et ne retient rien.
///
/// Cause du test instable `un_flux_indisponible_sert_le_cache_hors_ligne_…` : `tracing` met en
/// cache, par point d'émission (`warn!`…), l'intérêt des abonnés. Les tests tournent en
/// parallèle ; quand un autre test passe le premier par ce point d'émission **sans** abonné
/// local, le cache le marque « jamais » et l'abonné local d'un test qui capture ne le reçoit
/// plus (seuls les points déjà « toujours », comme ceux de hyper, passent). Tant qu'un seul
/// abonné est enregistré, `tracing-core` ne consulte que celui du thread qui enregistre le point
/// d'émission (`has_just_one`), donc `NoSubscriber` sur un thread de test nu. Un abonné global
/// permissif garde tous les points d'émission actifs ; l'abonné local, prioritaire sur son
/// thread, reçoit alors chaque événement.
pub(crate) fn keep_callsites_enabled() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Le registre accepte tout sans rien formater.
        let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry());
    });
}
