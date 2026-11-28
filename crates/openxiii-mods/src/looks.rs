//! Friendly names of the characters' models, for `look = "Jones"` in a
//! `[[map.<Map>.change]]` entry (placement.rs), and the "did you mean" helper of the mods' log.
//!
//! A look is a character model (`Mesh`) of the game's packages. It can be written in full
//! ("XIIIPersos.JonesM") or by one of the names below. Every model of XIIIPersos also answers to
//! its short name without the final "M" ("Spads" for XIIIPersos.SpadsM). character-models.md
//! (docs/modding) says who wears each model in the game.

/// (model, friendly names): the names a modder can write instead of the model's full name.
/// The first name is the one the log suggests.
pub const CHARACTER_LOOKS: &[(&str, &[&str])] = &[
    ("XIIIPersos.XIIIM", &["XIII", "Hero"]),
    ("XIIIPersos.XIIIMilitM", &["XIIIMilitary"]),
    ("XIIIPersos.XIIIBandagesM", &["XIIIBandages"]),
    ("XIIIPersos.XIIITatooM", &["XIIITattoo"]),
    ("XIIIPersos.JonesM", &["Jones", "MajorJones"]),
    ("XIIIPersos.SoireeM", &["JonesEvening", "JonesDress"]),
    ("XIIIPersosG.JonesMajM", &["JonesUniform"]),
    ("XIIIPersos.CarringtonM", &["Carrington", "General"]),
    ("XIIIPersos.MangousteM", &["Mongoose", "Mangouste"]),
    ("XIIIPersos.KimM", &["Kim", "KimRowland"]),
    ("XIIIPersos.AmosM", &["Amos", "ColonelAmos"]),
    ("XIIIPersos.McCallM", &["McCall", "ColonelMcCall"]),
    ("XIIIPersos.JohanssonM", &["Johansson", "DoctorJohansson"]),
    ("XIIIPersos.JessicaM", &["Jessica"]),
    ("XIIIPersos.PachaM", &["Pacha", "Edelbright"]),
    ("XIIIPersos.PamM", &["Pam", "Janet"]),
    ("XIIIPersos.QuinnM", &["Quinn"]),
    ("XIIIPersos.GalbrainM", &["Galbrain"]),
    ("XIIIPersos.SheridanM", &["Sheridan", "WalterSheridan"]),
    ("XIIIPersos.StandwellM", &["Standwell"]),
    ("XIIIPersos.StandKM", &["StandwellK"]),
    ("XIIIPersos.WaxM", &["Wax", "CalvinWax"]),
    ("XIIIPersos.WaxKM", &["WaxK"]),
    ("XIIIPersos.WillardM", &["Willard"]),
    ("XIIIPersos.WinslowM", &["Winslow"]),
    ("XIIIPersos.FouM", &["Fool", "Madman"]),
    ("XIIIPersos.DocM", &["Doc", "Doctor"]),
    ("XIIIPersos.CorsaireM", &["Corsaire", "Corsair"]),
    ("XIIIPersos.MiocheM", &["Mioche", "Kid"]),
    ("XIIIPersosG.MigM", &["Mig"]),
    ("XIIIPersos.GardienM", &["Guard", "PrisonGuard"]),
    ("XIIIPersos.GroflicM", &["FatCop", "Groflic"]),
    ("XIIIPersos.SpadsM", &["Spads", "SpadsSoldier", "Soldier"]),
    ("XIIIPersos.GiM", &["GI"]),
    ("XIIIPersos.FbiM", &["FBI", "FBIAgent"]),
    ("XIIIPersos.FBI2M", &["FBI2"]),
    ("XIIIPersos.KKKM", &["KKK", "Hooded"]),
    ("XIIIPersos.SMM", &["Sailor", "SubmarineSailor"]),
    ("XIIIPersos.WigM", &["Wig"]),
    ("XIIIPersos.Tueur1M", &["Killer1", "Tueur1"]),
    ("XIIIPersos.Tueur2M", &["Killer2", "Tueur2"]),
    ("XIIIPersos.Tueur3M", &["Killer3", "Tueur3"]),
    ("XIIIPersos.Tueur4M", &["Killer4", "Tueur4"]),
    ("XIIIPersos.Tueur5M", &["Killer5", "Tueur5"]),
    ("XIIIPersos.Tueur6M", &["Killer6", "Tueur6"]),
    ("XIIIPersos.Employee1M", &["Employee1"]),
    ("XIIIPersos.Employee2M", &["Employee2"]),
    ("XIIIPersos.Bank1M", &["Bank1"]),
    ("XIIIPersos.Bank2M", &["Bank2"]),
    ("XIIIPersos.HomM", &["Man"]),
    ("XIIIPersos.FemM", &["Woman"]),
    ("XIIIPersos.MecanoM", &["Mechanic", "Mecano"]),
    ("XIIIPersos.PlongeurM", &["Diver"]),
    ("XIIIPersos.LoucheM", &["Louche", "Shady"]),
    ("XIIIPersos.MrBonnetM", &["MrBonnet"]),
    ("XIIIPersos.SamM", &["Sam"]),
    ("XIIIPersos.SlaterM", &["Slater"]),
    ("XIIIPersos.SpanishM", &["Spanish"]),
    ("XIIIPersos.SurfM", &["Surfer"]),
    ("XIIIPersos.TimM", &["Tim"]),
    ("XIIIPersos.GaminM", &["John", "Gamin"]),
    ("XIIIPersos.FrenchyM", &["Roger", "Frenchy"]),
    ("XIIIPersos.Frenchy2M", &["Robert", "Frenchy2"]),
    ("XIIIPersos.ScandiM", &["Trystan", "Scandi"]),
    ("XIIIPersos.NiheiM", &["Densetsu", "Nihei"]),
    ("XIIIPersos.RastaM", &["Armael", "Rasta"]),
    ("XIIIPersos.DanhsuM", &["Shinyaku", "Danhsu"]),
    ("XIIIPersos.DeathM", &["Death"]),
    ("XIIIPersos.DogM", &["Dog"]),
    ("XIIIPersos.ratM", &["Rat"]),
    ("XIIIPersos.RequinM", &["Shark", "Requin"]),
    ("XIIIPersos.CanardM", &["Duck"]),
    ("XIIIPersos.mouetteM", &["Seagull"]),
    ("XIIIPersos.MouetSolM", &["SeagullOnGround"]),
    ("XIIIPersos.MantaM", &["Manta"]),
];

/// Who each look of [`CHARACTER_LOOKS`] is, in a few words, and the levels where the player
/// meets characters with it (map names, from the maps of the game; empty: not placed in the
/// story). Shown in docs/modding/names.md and in the names catalog. Every look of
/// CHARACTER_LOOKS has one (tested).
pub const LOOK_INFO: &[(&str, &str, &str)] = &[
    ("XIIIPersos.XIIIM", "XIII, the hero", "Banque01, Amos01, Hual02"),
    ("XIIIPersos.XIIIMilitM", "XIII in military clothes", "SPADS02a"),
    ("XIIIPersos.XIIIBandagesM", "XIII with bandages", ""),
    ("XIIIPersos.XIIITatooM", "XIII, another look", ""),
    ("XIIIPersos.JonesM", "Major Jones", "Banque01, SMarin01"),
    ("XIIIPersos.SoireeM", "Major Jones in an evening dress", "Bateau01"),
    ("XIIIPersosG.JonesMajM", "Major Jones in uniform", "Amos01, Toits01, Hual04c"),
    ("XIIIPersos.CarringtonM", "General Carrington", "Hual02, Hual04a, Kello01b"),
    ("XIIIPersos.MangousteM", "The Mongoose, the hired killer", "Plage00, Kello01a, Palace01"),
    ("XIIIPersos.KimM", "Kim Rowland", "Kello01b, SMarin01"),
    ("XIIIPersos.AmosM", "Colonel Amos", "Banque01, Amos01, USA02"),
    ("XIIIPersos.McCallM", "Colonel McCall", "Hual02, SPADS01, SPADS02b"),
    ("XIIIPersos.JohanssonM", "Doctor Johansson", "PRock01a, PRock03"),
    ("XIIIPersos.JessicaM", "Jessica", "Banque01"),
    ("XIIIPersos.PachaM", "Edelbright", "SMarin01, SMarin02"),
    ("XIIIPersos.PamM", "Janet, who finds XIII on the beach", "Plage00, Plage01"),
    ("XIIIPersos.QuinnM", "Quinn", "USA01, USA02, SSH101c"),
    ("XIIIPersos.GalbrainM", "Galbrain", "SSH101a, SSH101b"),
    ("XIIIPersos.SheridanM", "Walter Sheridan", "SMarin01, SSH101a, SSH102b"),
    ("XIIIPersos.StandwellM", "General Standwell", "Hual02, SPADS01, Sanc01"),
    ("XIIIPersos.StandKM", "General Standwell, another look", "Sanc02a"),
    ("XIIIPersos.WaxM", "Calvin Wax", "Sanc01, Sanc02a, SSH101a"),
    ("XIIIPersos.WaxKM", "Calvin Wax, another look", ""),
    ("XIIIPersos.WillardM", "Willard", "SPADS01, SPADS02a, Palace01"),
    ("XIIIPersos.WinslowM", "Winslow", "Banque01, Palace01"),
    ("XIIIPersos.FouM", "An asylum inmate", "PRock01a, PRock03"),
    ("XIIIPersos.DocM", "An asylum doctor", "PRock03"),
    ("XIIIPersos.CorsaireM", "An asylum nurse", "PRock03"),
    ("XIIIPersos.MiocheM", "The bald killer in an orange vest", "Plage01"),
    ("XIIIPersosG.MigM", "A guard in dark clothes", "Palace01, Sanc01, Bateau01"),
    ("XIIIPersos.GardienM", "A uniformed guard", "Banque01, PRock01a, PRock03"),
    ("XIIIPersos.GroflicM", "A heavy prison guard", "PRock01a"),
    ("XIIIPersos.SpadsM", "A SPADS soldier", "Plage00, Hual01a, Kello01a"),
    ("XIIIPersos.GiM", "A US Army soldier", "Hual01a, Hual02, SSH101a"),
    ("XIIIPersos.FbiM", "An FBI agent", "Banque01, Amos01, Palace01"),
    ("XIIIPersos.FBI2M", "An FBI agent, another look", "Amos01, Palace01"),
    ("XIIIPersos.KKKM", "A hooded cult member", "PRock03, Sanc02a, Sanc03"),
    ("XIIIPersos.SMM", "A submarine sailor", "Base01, SMarin01, SMarin02"),
    ("XIIIPersos.WigM", "A guest of the hotel and the Sanctuary", "Palace01, Sanc02a, Sanc03"),
    ("XIIIPersos.Tueur1M", "A hired killer (look 1)", "Plage00, Banque01, Kello01a"),
    ("XIIIPersos.Tueur2M", "A hired killer (look 2)", "Plage00, Banque01, Kello01a"),
    ("XIIIPersos.Tueur3M", "A hired killer (look 3)", "Amos01, Toits01, Kello01a"),
    ("XIIIPersos.Tueur4M", "A hired killer (look 4)", "Banque01, Amos01, Kello01a"),
    ("XIIIPersos.Tueur5M", "A hired killer (look 5)", "Amos01, Toits01, Kello01a"),
    ("XIIIPersos.Tueur6M", "A hired killer (look 6)", "Kello01a, Kello01b, PRock04a"),
    ("XIIIPersos.Employee1M", "An employee (man)", "Banque01, SSH101a, Bateau01"),
    ("XIIIPersos.Employee2M", "An employee (woman)", "PRock03, SSH101a, Bateau01"),
    ("XIIIPersos.Bank1M", "A bank clerk", "Banque01, Bateau01"),
    ("XIIIPersos.Bank2M", "A bank clerk, another look", "Banque01, Bateau01"),
    ("XIIIPersos.HomM", "A man", ""),
    ("XIIIPersos.FemM", "A woman", ""),
    ("XIIIPersos.MecanoM", "A mechanic", "SMarin01"),
    ("XIIIPersos.PlongeurM", "A diver", ""),
    ("XIIIPersos.LoucheM", "A shady man", ""),
    ("XIIIPersos.MrBonnetM", "Mr Bonnet", ""),
    ("XIIIPersos.SamM", "Sam", ""),
    ("XIIIPersos.SlaterM", "Slater", ""),
    ("XIIIPersos.SpanishM", "A Spanish man", ""),
    ("XIIIPersos.SurfM", "A surfer", ""),
    ("XIIIPersos.TimM", "Tim", ""),
    ("XIIIPersos.GaminM", "John, a multiplayer character", ""),
    ("XIIIPersos.FrenchyM", "Roger, a multiplayer character", ""),
    ("XIIIPersos.Frenchy2M", "Robert, a multiplayer character", ""),
    ("XIIIPersos.ScandiM", "Trystan, a multiplayer character", ""),
    ("XIIIPersos.NiheiM", "Densetsu, a multiplayer character", ""),
    ("XIIIPersos.RastaM", "Armael, a multiplayer character", ""),
    ("XIIIPersos.DanhsuM", "Shinyaku, a multiplayer character", ""),
    ("XIIIPersos.DeathM", "Death", ""),
    ("XIIIPersos.DogM", "A dog", ""),
    ("XIIIPersos.ratM", "A rat", "Amos01"),
    ("XIIIPersos.RequinM", "A shark", "SPADS01"),
    ("XIIIPersos.CanardM", "A duck", ""),
    ("XIIIPersos.mouetteM", "A seagull in flight", ""),
    ("XIIIPersos.MouetSolM", "A seagull on the ground", ""),
    ("XIIIPersos.MantaM", "A manta ray", ""),
];

/// (who it is, levels) of look `model` (LOOK_INFO).
pub fn look_info(model: &str) -> Option<(&'static str, &'static str)> {
    LOOK_INFO.iter().find(|(m, _, _)| m.eq_ignore_ascii_case(model)).map(|(_, w, l)| (*w, *l))
}

/// The model a look names: a full name ("Package.Name") as written, else a friendly name or a
/// model's short name (case ignored). Err: the closest friendly names, for "did you mean".
pub fn resolve(look: &str) -> Result<String, Vec<String>> {
    let look = look.trim();
    if look.contains('.') {
        return Ok(look.to_string());
    }
    let short = |mesh: &str| mesh.rsplit('.').next().unwrap_or(mesh).to_string();
    for (mesh, names) in CHARACTER_LOOKS {
        let s = short(mesh);
        let bare = s.strip_suffix('M').unwrap_or(&s);
        if names.iter().any(|n| n.eq_ignore_ascii_case(look)) || s.eq_ignore_ascii_case(look) || bare.eq_ignore_ascii_case(look) {
            return Ok(mesh.to_string());
        }
    }
    Err(closest(look, look_names(), 3))
}

/// Every friendly name, in table order.
pub fn look_names() -> impl Iterator<Item = &'static str> {
    CHARACTER_LOOKS.iter().flat_map(|(_, names)| names.iter().copied())
}

/// Up to `n` of `candidates` closest to `word` (case ignored): those it starts or is contained
/// in first, then by edit distance; nothing too far from it.
pub fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>, n: usize) -> Vec<String> {
    let w = word.to_lowercase();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for c in candidates {
        let l = c.to_lowercase();
        let score = if l == w {
            0
        } else if !w.is_empty() && (l.starts_with(&w) || w.starts_with(&l)) {
            1
        } else if w.len() >= 3 && l.contains(&w) {
            2
        } else {
            let d = distance(&w, &l);
            // At most about a third of the word wrong.
            if d * 3 > w.chars().count().max(3) {
                continue;
            }
            2 + d
        };
        if !scored.iter().any(|(_, s)| s.eq_ignore_ascii_case(c)) {
            scored.push((score, c.to_string()));
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    scored.into_iter().take(n).map(|(_, s)| s).collect()
}

/// Levenshtein distance (characters).
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// " (did you mean A, B?)", or nothing without suggestions.
pub fn did_you_mean(suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        String::new()
    } else {
        format!(" (did you mean {}?)", suggestions.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_short_and_full_names() {
        assert_eq!(resolve("Jones").as_deref(), Ok("XIIIPersos.JonesM"));
        assert_eq!(resolve("jones").as_deref(), Ok("XIIIPersos.JonesM"));
        assert_eq!(resolve("JonesM").as_deref(), Ok("XIIIPersos.JonesM"));
        assert_eq!(resolve("Mongoose").as_deref(), Ok("XIIIPersos.MangousteM"));
        assert_eq!(resolve("Tueur3").as_deref(), Ok("XIIIPersos.Tueur3M"));
        assert_eq!(resolve("rat").as_deref(), Ok("XIIIPersos.ratM"));
        assert_eq!(resolve("XIIIPersos.SpadsM").as_deref(), Ok("XIIIPersos.SpadsM"));
        assert_eq!(resolve(" XIII ").as_deref(), Ok("XIIIPersos.XIIIM"));
    }

    #[test]
    fn unknown_look_gives_suggestions() {
        let e = resolve("Jnes").unwrap_err();
        assert_eq!(e.first().map(String::as_str), Some("Jones"), "{e:?}");
        let e = resolve("Carington").unwrap_err();
        assert!(e.iter().any(|s| s == "Carrington"), "{e:?}");
        // Nothing close: no suggestion rather than a random one.
        assert!(resolve("Zzzzzzzzzz").unwrap_err().is_empty());
        assert_eq!(did_you_mean(&["A".into(), "B".into()]), " (did you mean A, B?)");
        assert_eq!(did_you_mean(&[]), "");
    }

    #[test]
    fn every_name_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for (mesh, names) in CHARACTER_LOOKS {
            assert!(mesh.contains('.'), "{mesh}");
            for n in *names {
                assert!(seen.insert(n.to_lowercase()), "{n} twice");
            }
        }
        // Each name resolves to its own model (no earlier model's short name takes it).
        for (mesh, names) in CHARACTER_LOOKS {
            for n in *names {
                assert_eq!(resolve(n).as_deref(), Ok(*mesh), "{n}");
            }
        }
    }
}
