//! Texts that OpenXIII adds to the menus (the main menu's new boxes, the Select level, Mods and
//! Co-op pages, the multiplayer pages' setting, the co-op character screen, the loading recap), in each language the game ships: English (.int), French (.frt), German (.det),
//! Italian (.itt) and Spanish (.est). The language is the game's own ([Engine.Engine] Language,
//! read by config.rs), so these texts always match the rest of the menu. Words the original game
//! already has (for example "Back", the weapons' names) are not here: they are read from the
//! installed game's localization files with [`Vm::original_text`].

use crate::Vm;

/// Language codes of the game's localization files, in the order of [`TABLE`]'s columns.
pub const LANGUAGES: [&str; 5] = ["int", "frt", "det", "itt", "est"];

/// A text added by OpenXIII. `{0}`, `{1}` are filled by [`fill`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Text {
    /// Main menu box (label under its onomatopoeia).
    MenuMods,
    MenuCoop,
    /// Entry of the Load page, title of the Select level page.
    SelectLevel,
    /// Select level page: the equipment page's buttons.
    All,
    None,
    StartLevel,
    /// Mods page.
    ModsTitle,
    MoreAbove,
    MoreBelow,
    NoMods,
    NoModsFolder,
    /// Row line: other mod, what both set.
    ConflictShort,
    /// After ConflictShort when there are more: count.
    ConflictMore,
    /// Version.
    Version,
    /// Version, author.
    VersionBy,
    /// Other mod, what both set (this mod's value wins).
    ConflictThisWins,
    /// Other mod, what both set (the other mod's value wins).
    ConflictOtherWins,
    HelpKeys,
    HelpPad,
    /// File, system error.
    CannotWrite,
    /// What two mods both set (openxiii-mods' Conflict::what, translated by kind).
    WhatValue,
    WhatClass,
    WhatKey,
    WhatTexture,
    WhatTextureReplacement,
    WhatModel,
    WhatModelReplacement,
    WhatCharacter,
    /// Map, what is replaced.
    WhatMapReplacement,
    /// Co-op page: its entries (New game, Continue and Load are the
    /// game's own words when the installed game has them: these are the fallbacks).
    SplitScreen,
    HostLanGame,
    JoinLanGame,
    NewGame,
    Continue,
    LoadGame,
    /// Multiplayer pages' setting.
    PlayersOnThisScreen,
    /// Next to an entry or a setting whose game mode does not exist yet.
    ComingSoon,
    /// Co-op page, joining a game on the local network: the address box, the row.
    CoopAddressBox,
    /// Co-op page: the name this player has in the LAN co-op chat.
    CoopName,
    /// Co-op page, choosing the game: the extra enemies setting and its first value.
    ExtraEnemies,
    ExtraEnemiesNone,
    /// Co-op page, split screen: how the screen is cut (split_layout.rs) and its three values.
    SplitLayout,
    SplitLayoutAuto,
    SplitLayoutHorizontal,
    SplitLayoutVertical,
    CoopJoinNow,
    /// Lockstep co-op: the game waits for the other machine.
    LockstepWaiting,
    /// Lockstep co-op: the guest is joining the host (meeting, settings and save sent).
    LockstepJoining,
    /// Lockstep co-op: the two machines' games are no longer the same.
    LockstepDiffer,
    /// Lockstep co-op: why a network game could not start or ended (a box over the Co-op page).
    LockstepNoGuest,
    LockstepNoHost,
    LockstepOtherFiles,
    LockstepOtherVersion,
    LockstepOtherMap,
    LockstepPortBusy,
    LockstepLanguage,
    LockstepBadAddress,
    LockstepOtherLeft,
    LockstepLinkLost,
    /// Lockstep co-op: the two machines' games went apart for good (the session ends).
    LockstepApart,
    LockstepFailed,
    /// Network play: told on the menu after the game's host left (or was lost).
    HostLeft,
    /// Co-op character screen: title, the variant button, the confirm button, the confirmed
    /// state, the wait for the other player, a player's label (number).
    CharacterTitle,
    /// Character screen in split screen: the title across the top of both halves.
    CharacterSelection,
    CharacterVariant,
    CharacterConfirm,
    CharacterReady,
    CharacterWaiting,
    CharacterPlayer,
    /// Character screen: a player's state, and the hints of their own device (keyboard for the
    /// first player, gamepad for the second): moving, acting, and once ready.
    CharacterChoosing,
    CharacterOtherReady,
    CharacterSpotSaved,
    CharacterCameraSaved,
    CharacterSpotRemoved,
    HintKeysMove,
    HintKeysAct,
    HintKeysReady,
    HintPadMove,
    HintPadAct,
    HintPadReady,
    /// Character screen: names of the roster's characters that are not one person's name.
    NameMongoose,
    NameKim,
    NameAdmiral,
    NameFbi,
    NameKiller,
    NameConspirator,
    NamePoliceman,
    /// Mods page: the built-in Co-op entry (shipped with the game) and its text.
    IncludedWithGame,
    /// Mods page: a row whose box differs from what the game loaded at startup (the line under
    /// its name, and the text at the bottom of the box).
    RestartNeeded,
    RestartNeededAbout,
    CoopEntryAbout,
    /// Controls page (XIIIMenuInputPC): entry of the gamepad settings page.
    Gamepad,
    /// Co-op page, hosting a LAN game: this computer's address (others), how to join, the hint
    /// at the bottom, its rows; joining: the line on top, the hint, the address box's example;
    /// in the game: the hosted game's address, a remote player who joined or left, the name of
    /// a remote player who kept the default one.
    LanHostAddress,
    LanHostOtherAddresses,
    LanHostNoAddress,
    LanHostHowToJoin,
    LanHostHint,
    ScreenSplit,
    ExtraEnemiesPerPlayer,
    LanJoinIntro,
    LanJoinHint,
    /// Co-op page, joining: the list of the games found on the network (lockstep_find.rs).
    CoopFoundSearching,
    CoopFoundOtherVersion,
    LanGameOpen,
    LanPlayerJoined,
    LanPlayerLeft,
    LanPlayerNumber,
    /// Video options page (video_menu.rs): the new lines and their choices.
    VideoDisplayMode,
    VideoWindowed,
    VideoBorderless,
    VideoExclusive,
    VideoRenderScale,
    VideoAntialiasing,
    VideoTextureFilter,
    /// Short form of VideoTextureFilter, when the full one does not fit in its box.
    VideoTextureFilterShort,
    /// The menus' size (UIScale).
    VideoUiScale,
    /// No anti-aliasing / no anisotropic filtering.
    VideoOff,
    /// Shadow quality line and its choices (Auto follows the resolution and the render scale).
    VideoShadows,
    VideoShadowsAuto,
    VideoShadowsLow,
    VideoShadowsHigh,
    VideoShadowsVeryHigh,
    /// Graphics line: the original game's look, or OpenXIII's visual improvements.
    VideoGraphics,
    VideoGraphicsOriginal,
    VideoGraphicsEnhanced,
    /// V-sync line (VSync), its short form when the full one does not fit, and its choices
    /// (Off; On: Classic, each image waits for the refresh).
    VideoVsync,
    VideoVsyncShort,
    VideoVsyncOn,
    VideoVsyncOff,
}

/// Every text, then its English, French, German, Italian and Spanish versions.
pub const TABLE: &[(Text, [&str; 5])] = &[
    (Text::MenuMods, ["Mods", "Mods", "Mods", "Mod", "Mods"]),
    (Text::MenuCoop, ["Co-op", "Co-op", "Koop", "Co-op", "Co-op"]),
    (Text::SelectLevel, ["Select level", "Sélectionner un niveau", "Levelauswahl", "Selezione livello", "Elegir nivel"]),
    (Text::All, ["All", "Tout", "Alles", "Tutto", "Todo"]),
    (Text::None, ["None", "Rien", "Nichts", "Niente", "Nada"]),
    (Text::VideoDisplayMode, ["Display", "Affichage", "Anzeige", "Schermo", "Pantalla"]),
    (Text::VideoWindowed, ["Window", "Fenêtre", "Fenster", "Finestra", "Ventana"]),
    (Text::VideoBorderless, ["Borderless", "Sans bordure", "Randlos", "Senza bordi", "Sin bordes"]),
    (Text::VideoExclusive, ["Full screen", "Plein écran", "Vollbild", "Schermo intero", "Completa"]),
    (Text::VideoRenderScale, ["Render scale", "Échelle", "Skalierung", "Scala", "Escala"]),
    (Text::VideoAntialiasing, ["Anti-aliasing", "Anticrénelage", "Kantenglättung", "Antialiasing", "Antialiasing"]),
    (Text::VideoTextureFilter, ["Anisotropic filtering", "Filtrage anisotrope", "Anisotrope Filterung", "Filtro anisotropico", "Filtrado anisotrópico"]),
    (Text::VideoTextureFilterShort, ["Aniso. filtering", "Filtrage aniso.", "Aniso. Filterung", "Filtro aniso.", "Filtrado aniso."]),
    (Text::VideoUiScale, ["Interface size", "Taille interface", "UI-Größe", "Dim. interfaccia", "Tamaño interfaz"]),
    (Text::VideoOff, ["Off", "Aucun", "Aus", "No", "No"]),
    (Text::VideoShadows, ["Shadows", "Ombres", "Schatten", "Ombre", "Sombras"]),
    (Text::VideoShadowsAuto, ["Auto", "Auto", "Auto", "Auto", "Auto"]),
    (Text::VideoShadowsLow, ["Low", "Basse", "Niedrig", "Bassa", "Baja"]),
    (Text::VideoShadowsHigh, ["High", "Haute", "Hoch", "Alta", "Alta"]),
    (Text::VideoShadowsVeryHigh, ["Very high", "Très haute", "Sehr hoch", "Molto alta", "Muy alta"]),
    (Text::VideoGraphics, ["Graphics", "Graphismes", "Grafik", "Grafica", "Gráficos"]),
    (Text::VideoGraphicsOriginal, ["Original", "Original", "Original", "Originale", "Originales"]),
    (Text::VideoGraphicsEnhanced, ["Enhanced", "Amélioré", "Verbessert", "Migliorata", "Mejorados"]),
    (Text::VideoVsync, ["V-sync", "Synchro verticale", "V-Sync", "Sincronia verticale", "Sincronía vertical"]),
    (Text::VideoVsyncShort, ["V-sync", "Synchro V.", "V-Sync", "V-Sync", "V-Sync"]),
    (Text::VideoVsyncOn, ["On", "Activée", "An", "Sì", "Sí"]),
    (Text::VideoVsyncOff, ["Off", "Désactivée", "Aus", "No", "No"]),
    (Text::StartLevel, ["Start level", "Lancer le niveau", "Level starten", "Avvia livello", "Iniciar nivel"]),
    (Text::ModsTitle, ["MODS", "MODS", "MODS", "MOD", "MODS"]),
    (Text::MoreAbove, ["more above", "plus haut", "weitere oben", "altre sopra", "más arriba"]),
    (Text::MoreBelow, ["more below", "plus bas", "weitere unten", "altre sotto", "más abajo"]),
    (
        Text::NoMods,
        [
            "No mods found in the Mods folder.",
            "Aucun mod dans le dossier Mods.",
            "Keine Mods im Ordner Mods gefunden.",
            "Nessuna mod nella cartella Mods.",
            "No hay mods en la carpeta Mods.",
        ],
    ),
    (Text::NoModsFolder, ["No Mods folder.", "Pas de dossier Mods.", "Kein Ordner Mods.", "Nessuna cartella Mods.", "No hay carpeta Mods."]),
    (Text::ConflictShort, ["Conflicts with {0}: {1}", "Conflit avec {0} : {1}", "Konflikt mit {0}: {1}", "Conflitto con {0}: {1}", "Conflicto con {0}: {1}"]),
    (Text::ConflictMore, [" (+{0} more)", " (+{0} autres)", " (+{0} weitere)", " (+{0} altri)", " (+{0} más)"]),
    (Text::Version, ["Version {0}", "Version {0}", "Version {0}", "Versione {0}", "Versión {0}"]),
    (Text::VersionBy, ["Version {0}, by {1}", "Version {0}, par {1}", "Version {0}, von {1}", "Versione {0}, di {1}", "Versión {0}, de {1}"]),
    (
        Text::ConflictThisWins,
        [
            "Conflicts with {0} on {1}: this mod's value is used.",
            "Conflit avec {0} sur {1} : la valeur de ce mod est utilisée.",
            "Konflikt mit {0} bei {1}: Der Wert dieses Mods wird verwendet.",
            "Conflitto con {0} su {1}: viene usato il valore di questa mod.",
            "Conflicto con {0} en {1}: se usa el valor de este mod.",
        ],
    ),
    (
        Text::ConflictOtherWins,
        [
            "Conflicts with {0} on {1}: {0}'s value is used.",
            "Conflit avec {0} sur {1} : la valeur de {0} est utilisée.",
            "Konflikt mit {0} bei {1}: Der Wert von {0} wird verwendet.",
            "Conflitto con {0} su {1}: viene usato il valore di {0}.",
            "Conflicto con {0} en {1}: se usa el valor de {0}.",
        ],
    ),
    (
        Text::HelpKeys,
        [
            "Enter or click: on / off.",
            "Entrée ou clic : activer / désactiver.",
            "Eingabe oder Klick: an / aus.",
            "Invio o clic: attiva / disattiva.",
            "Intro o clic: activar / desactivar.",
        ],
    ),
    (
        Text::HelpPad,
        [
            "A: on / off.",
            "A : activer / désactiver.",
            "A: an / aus.",
            "A: attiva / disattiva.",
            "A: activar / desactivar.",
        ],
    ),
    (
        Text::CannotWrite,
        [
            "Cannot write {0}: {1}",
            "Impossible d'écrire {0} : {1}",
            "{0} kann nicht geschrieben werden: {1}",
            "Impossibile scrivere {0}: {1}",
            "No se puede escribir {0}: {1}",
        ],
    ),
    (Text::WhatValue, ["value {0}", "valeur {0}", "Wert {0}", "valore {0}", "valor {0}"]),
    (Text::WhatClass, ["class {0}", "classe {0}", "Klasse {0}", "classe {0}", "clase {0}"]),
    (Text::WhatKey, ["key {0}", "touche {0}", "Taste {0}", "tasto {0}", "tecla {0}"]),
    (Text::WhatTexture, ["texture {0}", "texture {0}", "Textur {0}", "texture {0}", "textura {0}"]),
    (
        Text::WhatTextureReplacement,
        [
            "replacement of texture {0}",
            "remplacement de la texture {0}",
            "Ersatz der Textur {0}",
            "sostituzione della texture {0}",
            "sustitución de la textura {0}",
        ],
    ),
    (Text::WhatModel, ["model {0}", "modèle {0}", "Modell {0}", "modello {0}", "modelo {0}"]),
    (
        Text::WhatModelReplacement,
        ["replacement of model {0}", "remplacement du modèle {0}", "Ersatz des Modells {0}", "sostituzione del modello {0}", "sustitución del modelo {0}"],
    ),
    (Text::WhatCharacter, ["character {0}", "personnage {0}", "Figur {0}", "personaggio {0}", "personaje {0}"]),
    (
        Text::WhatMapReplacement,
        [
            "map {0}: replacement of {1}",
            "carte {0} : remplacement de {1}",
            "Karte {0}: Ersatz von {1}",
            "mappa {0}: sostituzione di {1}",
            "mapa {0}: sustitución de {1}",
        ],
    ),
    (Text::SplitScreen, ["Split-screen", "Écran partagé", "Geteilter Bildschirm", "Schermo condiviso", "Pantalla dividida"]),
    (Text::HostLanGame, ["Host LAN game", "Héberger une partie LAN", "LAN-Spiel erstellen", "Ospita partita LAN", "Crear partida LAN"]),
    (Text::JoinLanGame, ["Join LAN game", "Rejoindre une partie LAN", "LAN-Spiel beitreten", "Unisciti a partita LAN", "Unirse a partida LAN"]),
    (Text::NewGame, ["New game", "Nouvelle partie", "Neues Spiel", "Nuova partita", "Partida nueva"]),
    (Text::Continue, ["Continue", "Continuer", "Fortsetzen", "Continua", "Continuar"]),
    (Text::LoadGame, ["Load game", "Charger une partie", "Spiel laden", "Carica partita", "Cargar partida"]),
    (Text::PlayersOnThisScreen, ["Players on this screen", "Joueurs sur cet écran", "Spieler an diesem Bildschirm", "Giocatori su questo schermo", "Jugadores en esta pantalla"]),
    (Text::ComingSoon, ["coming soon", "bientôt", "bald", "presto", "pronto"]),
    (Text::CoopName, ["Co-op name", "Nom en co-op", "Koop-Name", "Nome in co-op", "Nombre en co-op"]),
    (Text::CoopAddressBox, ["IP address", "Adresse IP", "IP-Adresse", "Indirizzo IP", "Dirección IP"]),
    (Text::ExtraEnemies, ["Extra enemies", "Ennemis en plus", "Mehr Gegner", "Nemici in più", "Más enemigos"]),
    (Text::ExtraEnemiesNone, ["None", "Aucun", "Keine", "Nessuno", "Ninguno"]),
    (Text::SplitLayout, ["Split", "Partage", "Teilung", "Divisione", "División"]),
    (Text::SplitLayoutAuto, ["Auto", "Auto", "Auto", "Auto", "Auto"]),
    (Text::SplitLayoutHorizontal, ["Horizontal", "Horizontal", "Horizontal", "Orizzontale", "Horizontal"]),
    (Text::SplitLayoutVertical, ["Vertical", "Vertical", "Vertikal", "Verticale", "Vertical"]),
    (Text::CoopJoinNow, ["Join", "Rejoindre", "Beitreten", "Unisciti", "Unirse"]),
    (Text::LockstepWaiting, ["Waiting for the other player...", "En attente de l'autre joueur...", "Warten auf den anderen Spieler...", "In attesa dell'altro giocatore...", "Esperando al otro jugador..."]),
    (Text::LockstepJoining, ["Joining the host...", "Connexion à l'hôte...", "Verbindung zum Host...", "Connessione all'host...", "Conectando con el anfitrión..."]),
    (Text::LockstepDiffer, ["The two games differ", "Les deux parties ne sont plus pareilles", "Die beiden Spiele weichen ab", "Le due partite non coincidono", "Las dos partidas no coinciden"]),
    (Text::LockstepNoGuest, ["No other player came.", "Aucun autre joueur n'est venu.", "Kein anderer Spieler ist gekommen.", "Nessun altro giocatore è arrivato.", "No ha llegado ningún otro jugador."]),
    (Text::LockstepNoHost, ["The host does not answer. Check the address and that the host is waiting.", "L'hôte ne répond pas. Vérifiez l'adresse et que l'hôte attend.", "Der Host antwortet nicht. Prüfe die Adresse und ob der Host wartet.", "L'host non risponde. Controlla l'indirizzo e che l'host sia in attesa.", "El anfitrión no responde. Comprueba la dirección y que el anfitrión esté esperando."]),
    (Text::LockstepOtherFiles, ["The other computer has other game files or other mods.", "L'autre PC n'a pas les mêmes fichiers du jeu ou les mêmes mods.", "Der andere PC hat andere Spieldateien oder andere Mods.", "L'altro PC ha file di gioco o mod diversi.", "El otro PC tiene otros archivos del juego u otros mods."]),
    (Text::LockstepOtherVersion, ["The other computer has another version of OpenXIII.", "L'autre PC n'a pas la même version d'OpenXIII.", "Der andere PC hat eine andere OpenXIII-Version.", "L'altro PC ha un'altra versione di OpenXIII.", "El otro PC tiene otra versión de OpenXIII."]),
    (Text::LockstepOtherMap, ["The host's map is not the same as this one.", "La carte de l'hôte n'est pas la même que celle-ci.", "Die Karte des Hosts ist eine andere als diese.", "La mappa dell'host non è uguale a questa.", "El mapa del anfitrión no es igual que este."]),
    (Text::LockstepPortBusy, ["The network port is already used by another program.", "Le port réseau est déjà utilisé par un autre programme.", "Der Netzwerkport wird bereits von einem anderen Programm verwendet.", "La porta di rete è già usata da un altro programma.", "El puerto de red ya lo usa otro programa."]),
    (Text::LockstepLanguage, ["The host plays in a language that is not installed here.", "L'hôte joue dans une langue qui n'est pas installée ici.", "Der Host spielt in einer Sprache, die hier nicht installiert ist.", "L'host gioca in una lingua non installata qui.", "El anfitrión juega en un idioma que no está instalado aquí."]),
    (Text::LockstepBadAddress, ["This address is not valid.", "Cette adresse n'est pas valide.", "Diese Adresse ist ungültig.", "Questo indirizzo non è valido.", "Esta dirección no es válida."]),
    (Text::LockstepOtherLeft, ["The other player left the game.", "L'autre joueur a quitté la partie.", "Der andere Spieler hat das Spiel verlassen.", "L'altro giocatore ha lasciato la partita.", "El otro jugador ha abandonado la partida."]),
    (Text::LockstepLinkLost, ["The connection with the other player was lost.", "La connexion avec l'autre joueur est perdue.", "Die Verbindung zum anderen Spieler ist abgebrochen.", "La connessione con l'altro giocatore si è interrotta.", "Se perdió la conexión con el otro jugador."]),
    (Text::LockstepApart, ["The game went out of sync with the other player.", "La partie s'est désynchronisée avec l'autre joueur.", "Das Spiel ist mit dem anderen Spieler nicht mehr synchron.", "La partita non è più sincronizzata con l'altro giocatore.", "La partida se ha desincronizado con el otro jugador."]),
    (Text::LockstepFailed, ["The network game could not start.", "La partie en réseau n'a pas pu commencer.", "Das Netzwerkspiel konnte nicht beginnen.", "La partita in rete non è potuta iniziare.", "La partida en red no ha podido empezar."]),
    (Text::HostLeft, ["The host left the game.", "L'hôte a quitté la partie.", "Der Host hat das Spiel verlassen.", "L'host ha lasciato la partita.", "El anfitrión ha abandonado la partida."]),
    (Text::CharacterTitle, ["Choose a character", "Choix du personnage", "Figur wählen", "Scelta del personaggio", "Elección de personaje"]),
    (Text::CharacterSelection, ["Character Selection", "Sélection des personnages", "Figurenauswahl", "Selezione dei personaggi", "Selección de personajes"]),
    (Text::CharacterVariant, ["Variant", "Variante", "Variante", "Variante", "Variante"]),
    (Text::CharacterConfirm, ["Confirm", "Valider", "Bestätigen", "Conferma", "Confirmar"]),
    (Text::CharacterReady, ["Ready", "Prêt", "Bereit", "Pronto", "Listo"]),
    (Text::CharacterWaiting, ["Waiting for the other player", "En attente de l'autre joueur", "Warten auf den anderen Spieler", "In attesa dell'altro giocatore", "Esperando al otro jugador"]),
    (Text::CharacterPlayer, ["Player {0}", "Joueur {0}", "Spieler {0}", "Giocatore {0}", "Jugador {0}"]),
    (Text::CharacterChoosing, ["Choosing", "Choix en cours", "Auswahl läuft", "Scelta in corso", "Eligiendo"]),
    (Text::CharacterOtherReady, ["The other player is ready", "L'autre joueur est prêt", "Der andere Spieler ist bereit", "L'altro giocatore è pronto", "El otro jugador está listo"]),
    (Text::CharacterSpotSaved, ["Character spot saved", "Place du personnage enregistrée", "Platz der Figur gespeichert", "Posizione del personaggio salvata", "Posición del personaje guardada"]),
    (Text::CharacterCameraSaved, ["Camera saved", "Caméra enregistrée", "Kamera gespeichert", "Telecamera salvata", "Cámara guardada"]),
    (Text::CharacterSpotRemoved, ["Character spot removed", "Place du personnage effacée", "Platz der Figur gelöscht", "Posizione del personaggio cancellata", "Posición del personaje borrada"]),
    (Text::HintKeysMove, ["Up/Down   Left/Right", "Haut/Bas   Gauche/Droite", "Auf/Ab   Links/Rechts", "Su/Giù   Sinistra/Destra", "Arriba/Abajo   Izq./Der."]),
    (Text::HintKeysAct, ["Enter: ready   Esc: back", "Entrée : prêt   Échap : retour", "Eingabe: bereit   Esc: zurück", "Invio: pronto   Esc: indietro", "Intro: listo   Esc: volver"]),
    (Text::HintKeysReady, ["Esc: cancel ready", "Échap : annuler prêt", "Esc: Bereit aufheben", "Esc: annulla pronto", "Esc: cancelar listo"]),
    (Text::HintPadMove, ["D-pad up/down   left/right", "Croix haut/bas   gauche/droite", "Kreuz auf/ab   links/rechts", "Croce su/giù   sinistra/destra", "Cruceta arriba/abajo   izq./der."]),
    (Text::HintPadAct, ["A: ready   B: back", "A : prêt   B : retour", "A: bereit   B: zurück", "A: pronto   B: indietro", "A: listo   B: volver"]),
    (Text::HintPadReady, ["B: cancel ready", "B : annuler prêt", "B: Bereit aufheben", "B: annulla pronto", "B: cancelar listo"]),
    (Text::NameMongoose, ["The Mongoose", "La Mangouste", "Mongoose", "La Mangusta", "La Mangosta"]),
    (Text::NameKim, ["Kim Rowland", "Kim Rowland", "Kim Rowland", "Kim Rowland", "Kim Rowland"]),
    (Text::NameAdmiral, ["Edelbright", "Edelbright", "Edelbright", "Edelbright", "Edelbright"]),
    (Text::NameFbi, ["FBI agent", "Agent du FBI", "FBI-Agent", "Agente dell'FBI", "Agente del FBI"]),
    (Text::NameKiller, ["Killer", "Tueur", "Killer", "Killer", "Asesino"]),
    (Text::NameConspirator, ["Hooded conspirator", "Conjuré encagoulé", "Vermummter Verschwörer", "Cospiratore incappucciato", "Conspirador encapuchado"]),
    (Text::NamePoliceman, ["Policeman", "Policier", "Polizist", "Poliziotto", "Policía"]),
    (Text::IncludedWithGame, ["Included with the game", "Fourni avec le jeu", "Im Spiel enthalten", "Incluso nel gioco", "Incluido con el juego"]),
    (Text::RestartNeeded, ["Restart needed", "Redémarrage nécessaire", "Neustart nötig", "Riavvio necessario", "Reinicio necesario"]),
    (
        Text::RestartNeededAbout,
        [
            "This change applies the next time the game starts.",
            "Ce changement s'appliquera au prochain lancement du jeu.",
            "Diese Änderung gilt ab dem nächsten Start des Spiels.",
            "Questa modifica vale dal prossimo avvio del gioco.",
            "Este cambio se aplica la próxima vez que se inicie el juego.",
        ],
    ),
    (
        Text::CoopEntryAbout,
        [
            "Co-op box of the main menu and \"Players on this screen\" on the multiplayer pages. Unchecked: the menus are the original game's.",
            "Case Co-op du menu principal et réglage « Joueurs sur cet écran » des pages multijoueur. Décoché : les menus sont ceux du jeu d'origine.",
            "Koop-Feld im Hauptmenü und \"Spieler an diesem Bildschirm\" auf den Mehrspielerseiten. Ausgeschaltet: die Menüs sind die des Originalspiels.",
            "Riquadro Co-op del menu principale e \"Giocatori su questo schermo\" nelle pagine multigiocatore. Disattivato: i menu sono quelli del gioco originale.",
            "Casilla Co-op del menú principal y \"Jugadores en esta pantalla\" en las páginas multijugador. Desactivado: los menús son los del juego original.",
        ],
    ),
    (Text::Gamepad, ["Gamepad", "Manette", "Gamepad", "Controller", "Mando"]),
    (Text::LanHostAddress, ["You are hosting the game. Your address: {0}", "Tu héberges la partie. Ton adresse : {0}", "Du bist der Host. Deine Adresse: {0}", "Stai ospitando la partita. Il tuo indirizzo: {0}", "Eres el anfitrión. Tu dirección: {0}"]),
    (Text::LanHostOtherAddresses, ["(other network: {0})", "(autre réseau : {0})", "(anderes Netzwerk: {0})", "(altra rete: {0})", "(otra red: {0})"]),
    (Text::LanHostNoAddress, ["You are hosting the game. No network found on this PC.", "Tu héberges la partie. Aucun réseau trouvé sur ce PC.", "Du bist der Host. Kein Netzwerk auf diesem PC gefunden.", "Stai ospitando la partita. Nessuna rete trovata su questo PC.", "Eres el anfitrión. No se encontró ninguna red en este PC."]),
    (Text::LanHostHowToJoin, ["The other PC goes to {0} > {1} and types this address.", "L'autre PC va dans {0} > {1} et tape cette adresse.", "Der andere PC öffnet {0} > {1} und gibt diese Adresse ein.", "L'altro PC va su {0} > {1} e digita questo indirizzo.", "El otro PC va a {0} > {1} y escribe esta dirección."]),
    (Text::LanHostHint, ["The game starts once your guest has joined. If Windows asks for network access, allow it.", "La partie commence dès que ton invité t'a rejoint. Si Windows demande l'accès au réseau, accepte.", "Das Spiel beginnt, sobald dein Gast beigetreten ist. Wenn Windows nach dem Netzwerkzugriff fragt, erlaube ihn.", "La partita inizia quando il tuo ospite si è unito. Se Windows chiede l'accesso alla rete, consentilo.", "La partida empieza cuando tu invitado se haya unido. Si Windows pide acceso a la red, acéptalo."]),
    (Text::ScreenSplit, ["Screen split", "Découpage de l'écran", "Bildschirmteilung", "Divisione dello schermo", "División de pantalla"]),
    (Text::ExtraEnemiesPerPlayer, ["Extra enemies per player", "Ennemis en plus par joueur", "Mehr Gegner pro Spieler", "Nemici in più per giocatore", "Más enemigos por jugador"]),
    (Text::LanJoinIntro, ["Choose a game below, or type the host's address.", "Choisis une partie plus bas, ou tape l'adresse de l'hôte.", "Wähle unten ein Spiel oder gib die Adresse des Hosts ein.", "Scegli una partita qui sotto o digita l'indirizzo dell'host.", "Elige una partida abajo o escribe la dirección del anfitrión."]),
    (Text::CoopFoundSearching, ["Looking for co-op games on the network...", "Recherche de parties co-op sur le réseau...", "Suche nach Koop-Spielen im Netzwerk...", "Ricerca di partite co-op in rete...", "Buscando partidas cooperativas en la red..."]),
    (Text::CoopFoundOtherVersion, ["(other version)", "(autre version)", "(andere Version)", "(altra versione)", "(otra versión)"]),
    (Text::LanJoinHint, ["The host must have started their game first.", "L'hôte doit avoir lancé sa partie avant.", "Der Host muss sein Spiel vorher gestartet haben.", "L'host deve aver già avviato la partita.", "El anfitrión debe haber iniciado su partida antes."]),
    (Text::LanGameOpen, ["LAN game open: {0}", "Partie LAN ouverte : {0}", "LAN-Spiel offen: {0}", "Partita LAN aperta: {0}", "Partida LAN abierta: {0}"]),
    (Text::LanPlayerJoined, ["{0} joined the game", "{0} a rejoint la partie", "{0} ist dem Spiel beigetreten", "{0} si è unito alla partita", "{0} se ha unido a la partida"]),
    (Text::LanPlayerLeft, ["{0} left the game", "{0} a quitté la partie", "{0} hat das Spiel verlassen", "{0} ha lasciato la partita", "{0} ha salido de la partida"]),
    (Text::LanPlayerNumber, ["Player {0}", "Joueur {0}", "Spieler {0}", "Giocatore {0}", "Jugador {0}"]),
];

/// Names of the character screen's variants (character_screen::ROSTER), in the order of
/// [`LANGUAGES`].
pub const VARIANT_NAMES: &[(&str, [&str; 5])] = &[
    ("jacket", ["Jacket", "Blouson", "Jacke", "Giubbotto", "Cazadora"]),
    ("bandages", ["Bandages", "Bandages", "Verbände", "Bende", "Vendajes"]),
    ("military", ["Military", "Militaire", "Militär", "Militare", "Militar"]),
    ("general", ["General's uniform", "Uniforme de général", "Generalsuniform", "Uniforme da generale", "Uniforme de general"]),
    ("torn", ["Torn clothes", "Vêtements déchirés", "Zerrissene Kleidung", "Vestiti strappati", "Ropa rasgada"]),
    ("blouse", ["Blouse", "Chemisier", "Bluse", "Camicetta", "Blusa"]),
    ("evening_dress", ["Evening dress", "Robe de soirée", "Abendkleid", "Abito da sera", "Vestido de noche"]),
    ("camouflage", ["Camouflage", "Treillis", "Tarnanzug", "Mimetica", "Camuflaje"]),
    ("burgundy_suit", ["Burgundy suit", "Costume bordeaux", "Bordeauxroter Anzug", "Abito bordeaux", "Traje burdeos"]),
    ("cream_suit", ["Cream suit", "Costume crème", "Cremefarbener Anzug", "Abito crema", "Traje crema"]),
    ("grey_suit", ["Grey suit", "Costume gris", "Grauer Anzug", "Abito grigio", "Traje gris"]),
    ("tuxedo", ["Tuxedo", "Smoking", "Smoking", "Smoking", "Esmoquin"]),
    ("uniform", ["Uniform", "Uniforme", "Uniform", "Uniforme", "Uniforme"]),
    ("robe", ["Robe", "Toge", "Robe", "Tunica", "Túnica"]),
    ("standard", ["Standard", "Standard", "Standard", "Standard", "Estándar"]),
    ("jungle", ["Jungle", "Jungle", "Dschungel", "Giungla", "Selva"]),
    ("rolled_sleeves", ["Rolled-up sleeves", "Manches retroussées", "Hochgekrempelte Ärmel", "Maniche arrotolate", "Mangas remangadas"]),
    ("killer1", ["Killer 1", "Tueur 1", "Killer 1", "Killer 1", "Asesino 1"]),
    ("killer2", ["Killer 2", "Tueur 2", "Killer 2", "Killer 2", "Asesino 2"]),
    ("killer3", ["Killer 3", "Tueur 3", "Killer 3", "Killer 3", "Asesino 3"]),
    ("killer4", ["Killer 4", "Tueur 4", "Killer 4", "Killer 4", "Asesino 4"]),
    ("cop", ["Cop", "Flic", "Bulle", "Sbirro", "Poli"]),
    ("fat_cop", ["Fat cop", "Gros flic", "Dicker Bulle", "Sbirro grasso", "Poli gordo"]),
];

/// Name of a variant of the character screen in language `lang` (empty for an unknown key).
pub fn variant_name(lang: &str, key: &str) -> &'static str {
    let column = LANGUAGES.iter().position(|l| l.eq_ignore_ascii_case(lang)).unwrap_or(0);
    VARIANT_NAMES.iter().find(|(k, _)| *k == key).map_or("", |(_, v)| v[column])
}

/// `t` in language `lang` (a localization extension such as "frt"); English for any other.
pub fn text(lang: &str, t: Text) -> &'static str {
    let column = LANGUAGES.iter().position(|l| l.eq_ignore_ascii_case(lang)).unwrap_or(0);
    TABLE.iter().find(|(k, _)| *k == t).map_or("", |(_, v)| v[column])
}

/// `template` with `{0}`, `{1}`… replaced by `args`.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = template.to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

/// What two mods both set (openxiii-mods writes it in English: "value XIII.Magnum.ReloadCount",
/// "key F5", "map Plage01: replacement of A.B"…), in language `lang`. Unknown forms stay as they are.
pub fn what(lang: &str, what: &str) -> String {
    if let Some(rest) = what.strip_prefix("map ") {
        if let Some((map, from)) = rest.split_once(": replacement of ") {
            return fill(text(lang, Text::WhatMapReplacement), &[map, from]);
        }
    }
    const KINDS: [(&str, Text); 8] = [
        ("replacement of texture ", Text::WhatTextureReplacement),
        ("replacement of model ", Text::WhatModelReplacement),
        ("value ", Text::WhatValue),
        ("class ", Text::WhatClass),
        ("key ", Text::WhatKey),
        ("texture ", Text::WhatTexture),
        ("model ", Text::WhatModel),
        ("character ", Text::WhatCharacter),
    ];
    for (prefix, t) in KINDS {
        if let Some(name) = what.strip_prefix(prefix) {
            return fill(text(lang, t), &[name]);
        }
    }
    what.to_string()
}

impl Vm {
    /// Text `t` added by OpenXIII, in the game's language.
    pub fn added_text(&self, t: Text) -> &'static str {
        text(self.text_language(), t)
    }

    /// A text of the installed game's localization ("Package.<language>", [section] key), if
    /// present (Localize without the "No localization" trace).
    pub fn original_text(&mut self, section: &str, key: &str, package: &str) -> Option<String> {
        self.config.as_mut()?.localize2(section, key, package).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// Name of line `i` of select_level::WEAPONS in the game's language: the game's own pickup
    /// message or item name, the English name when the game has none.
    pub fn weapon_label(&mut self, i: usize) -> String {
        let w = &crate::select_level::WEAPONS[i];
        self.original_text(w.label.0, w.label.1, "XIII").unwrap_or_else(|| w.name_value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Placeholders `{n}` of a text.
    fn placeholders(s: &str) -> Vec<usize> {
        let mut v: Vec<usize> = (0..4).filter(|i| s.contains(&format!("{{{i}}}"))).collect();
        v.dedup();
        v
    }

    #[test]
    fn every_language_has_every_text() {
        let all = [
            Text::MenuMods,
            Text::MenuCoop,
            Text::SelectLevel,
            Text::All,
            Text::None,
            Text::StartLevel,
            Text::ModsTitle,
            Text::MoreAbove,
            Text::MoreBelow,
            Text::NoMods,
            Text::NoModsFolder,
            Text::ConflictShort,
            Text::ConflictMore,
            Text::Version,
            Text::VersionBy,
            Text::ConflictThisWins,
            Text::ConflictOtherWins,
            Text::HelpKeys,
            Text::HelpPad,
            Text::CannotWrite,
            Text::WhatValue,
            Text::WhatClass,
            Text::WhatKey,
            Text::WhatTexture,
            Text::WhatTextureReplacement,
            Text::WhatModel,
            Text::WhatModelReplacement,
            Text::WhatCharacter,
            Text::WhatMapReplacement,
            Text::SplitScreen,
            Text::HostLanGame,
            Text::JoinLanGame,
            Text::NewGame,
            Text::Continue,
            Text::LoadGame,
            Text::PlayersOnThisScreen,
            Text::ComingSoon,
            Text::CoopAddressBox,
            Text::CoopName,
            Text::ExtraEnemies,
            Text::ExtraEnemiesNone,
            Text::SplitLayout,
            Text::SplitLayoutAuto,
            Text::SplitLayoutHorizontal,
            Text::SplitLayoutVertical,
            Text::CoopJoinNow,
            Text::LockstepWaiting,
            Text::LockstepJoining,
            Text::LockstepDiffer,
            Text::LockstepNoGuest,
            Text::LockstepNoHost,
            Text::LockstepOtherFiles,
            Text::LockstepOtherVersion,
            Text::LockstepOtherMap,
            Text::LockstepPortBusy,
            Text::LockstepLanguage,
            Text::LockstepBadAddress,
            Text::LockstepOtherLeft,
            Text::LockstepLinkLost,
            Text::LockstepApart,
            Text::LockstepFailed,
            Text::HostLeft,
            Text::CharacterTitle,
            Text::CharacterSelection,
            Text::CharacterVariant,
            Text::CharacterConfirm,
            Text::CharacterReady,
            Text::CharacterWaiting,
            Text::CharacterPlayer,
            Text::CharacterChoosing,
            Text::CharacterOtherReady,
            Text::CharacterSpotSaved,
            Text::CharacterCameraSaved,
            Text::CharacterSpotRemoved,
            Text::HintKeysMove,
            Text::HintKeysAct,
            Text::HintKeysReady,
            Text::HintPadMove,
            Text::HintPadAct,
            Text::HintPadReady,
            Text::NameMongoose,
            Text::NameKim,
            Text::NameAdmiral,
            Text::NameFbi,
            Text::NameKiller,
            Text::NameConspirator,
            Text::NamePoliceman,
            Text::IncludedWithGame,
            Text::RestartNeeded,
            Text::RestartNeededAbout,
            Text::CoopEntryAbout,
            Text::Gamepad,
            Text::VideoDisplayMode,
            Text::VideoWindowed,
            Text::VideoBorderless,
            Text::VideoExclusive,
            Text::VideoRenderScale,
            Text::VideoAntialiasing,
            Text::VideoTextureFilter,
            Text::VideoTextureFilterShort,
            Text::VideoUiScale,
            Text::VideoOff,
            Text::VideoShadows,
            Text::VideoShadowsAuto,
            Text::VideoShadowsLow,
            Text::VideoShadowsHigh,
            Text::VideoShadowsVeryHigh,
            Text::VideoGraphics,
            Text::VideoGraphicsOriginal,
            Text::VideoGraphicsEnhanced,
            Text::VideoVsync,
            Text::VideoVsyncShort,
            Text::VideoVsyncOn,
            Text::VideoVsyncOff,
            Text::LanHostAddress,
            Text::LanHostOtherAddresses,
            Text::LanHostNoAddress,
            Text::LanHostHowToJoin,
            Text::LanHostHint,
            Text::ScreenSplit,
            Text::ExtraEnemiesPerPlayer,
            Text::LanJoinIntro,
            Text::LanJoinHint,
            Text::CoopFoundSearching,
            Text::CoopFoundOtherVersion,
            Text::LanGameOpen,
            Text::LanPlayerJoined,
            Text::LanPlayerLeft,
            Text::LanPlayerNumber,
        ];
        assert_eq!(TABLE.len(), all.len(), "a text is missing from the test's list or the table has a duplicate");
        for t in all {
            let rows: Vec<_> = TABLE.iter().filter(|(k, _)| *k == t).collect();
            assert_eq!(rows.len(), 1, "{t:?} must be in the table once");
            let english = placeholders(rows[0].1[0]);
            for (lang, s) in LANGUAGES.iter().zip(rows[0].1) {
                assert!(!s.trim().is_empty(), "{t:?} is empty in {lang}");
                assert_eq!(placeholders(s), english, "{t:?} in {lang} must use the same placeholders as English");
                assert_eq!(text(lang, t), s);
            }
        }
    }

    #[test]
    fn unknown_language_falls_back_to_english() {
        assert_eq!(text("xyz", Text::MoreBelow), "more below");
        assert_eq!(text("FRT", Text::MoreBelow), "plus bas");
    }

    #[test]
    fn conflict_kinds_are_translated() {
        assert_eq!(what("frt", "value XIII.Magnum.ReloadCount"), "valeur XIII.Magnum.ReloadCount");
        assert_eq!(what("det", "key F5"), "Taste F5");
        assert_eq!(what("frt", "replacement of texture A.B"), "remplacement de la texture A.B");
        assert_eq!(what("itt", "map Plage01: replacement of XIII.Gun"), "mappa Plage01: sostituzione di XIII.Gun");
        assert_eq!(what("est", "something new"), "something new");
        assert_eq!(what("int", "model Mod.Gun"), "model Mod.Gun");
        assert_eq!(fill(text("frt", Text::ConflictOtherWins), &["B", "x"]), "Conflit avec B sur x : la valeur de B est utilisée.");
    }

    #[test]
    fn the_co_op_host_waits_for_its_guest() {
        // Lockstep: the host waits for its guest before the game starts; no guest arrives later.
        for lang in ["int", "frt", "det", "itt", "est"] {
            let t = text(lang, Text::LanHostHint);
            assert!(!t.is_empty(), "{lang}");
        }
        assert!(text("int", Text::LanHostHint).starts_with("The game starts once your guest has joined."));
        assert!(text("frt", Text::LanHostHint).starts_with("La partie commence dès que ton invité t'a rejoint."));
        assert!(!text("int", Text::LanHostHint).contains("during the game"));
    }

}
