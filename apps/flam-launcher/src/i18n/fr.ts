import { ownText } from "./lookup";

const storyCount = (n: number) => (n <= 1 ? `${n} histoire` : `${n} histoires`);

/**
 * Textes de l'interface en français. Tout texte affiché passe par ce fichier pour
 * préparer une future traduction : une autre langue fournira un objet du même type.
 */
export const fr = {
  app: {
    name: "Flam Launcher",
    skipToContent: "Aller au contenu",
    sourcesNav: "Sources d'histoires",
  },
  nav: {
    label: "Navigation",
    home: "Accueil",
    sources: "Sources",
    settings: "Paramètres",
    library: "Bibliothèque",
    device: "Ma Flam",
    podcasts: "Podcasts",
  },
  errors: {
    title: (kind: string) =>
      ownText(
        {
          validation: "Valeur refusée",
          not_found: "Élément introuvable",
          io: "Erreur d'accès aux fichiers",
          config_version: "Configuration trop récente",
          cache_dir_in_use: "Dossier de cache déjà utilisé",
          cache_busy: "Cache occupé",
          network: "Réseau indisponible",
          catalog_invalid: "Catalogue illisible",
          folder_unavailable: "Dossier introuvable",
          tts_no_voice: "Aucune voix française",
          tts_unavailable: "Synthèse vocale indisponible",
          tts_failed: "Titres non lus à voix haute",
        },
        kind,
      ) ?? "Une erreur est survenue",
    loadFailed: "Impossible de charger les données.",
    /** Erreur de forme inattendue (ni texte ni `Error`). */
    unknown: "Erreur inconnue",
    retry: "Réessayer",
    /**
     * Message montré pour un code d'erreur du convertisseur (`TaskError.code`, `kind` « conversion »),
     * à la place du texte brut du backend. Code absent de la table : le message du backend.
     */
    conversion: {
      unsupported_format:
        "Cette histoire utilise un format que le launcher ne sait pas encore convertir (format d'image non pris en charge, par exemple BMP).",
    } as Record<string, string>,
    /** Détail du backend, sous le texte traduit d'un échec `tts_failed` (panneau des tâches). */
    detail: (message: string) => `Détail : ${message}`,
    /**
     * Message montré pour une tâche (ou une création de pack) en échec, selon son `kind`, à la
     * place du texte du backend. `kind` absent de la table : le message du backend.
     */
    task: {
      tts_no_voice:
        "Aucune voix française n'est installée sur cet ordinateur : le pack n'a pas été créé. Ajoutez une voix dans Windows (Paramètres > Heure et langue > Voix), ou décochez « Lire les titres à voix haute ».",
      tts_unavailable:
        "La synthèse vocale de Windows n'est pas disponible sur cet ordinateur : le pack n'a pas été créé. Réessayez plus tard, ou décochez « Lire les titres à voix haute ».",
      tts_failed:
        "Les titres n'ont pas pu être lus à voix haute : le pack n'a pas été créé. Réessayez, vérifiez les voix de Windows (Paramètres > Heure et langue > Voix), ou décochez « Lire les titres à voix haute ».",
    } as Record<string, string>,
  },
  /** Titres lus à voix haute dans les packs de podcasts (#462). */
  speech: {
    checkbox: "Lire les titres à voix haute",
    checkboxHint:
      "La Flam dira le titre du pack, de chaque catégorie et de chaque épisode quand on le choisit dans le carrousel. Voix de Windows, sur cet ordinateur seulement.",
    loading: "Recherche des voix françaises…",
    noVoice:
      "Aucune voix française n'est installée sur cet ordinateur : les titres ne peuvent pas être lus à voix haute. Pour en ajouter une : Paramètres de Windows > Heure et langue > Voix > Ajouter des voix, puis rouvrez cette page.",
    listFailed:
      "Les voix de Windows n'ont pas pu être listées : les titres ne peuvent pas être lus à voix haute pour l'instant.",
    voiceUsed: (name: string) => `Voix : ${name} (à changer dans les Paramètres).`,
    voiceOfComposition: (name: string) => `Voix : ${name} (celle de la composition).`,
    voiceGone: (name: string) =>
      `La voix choisie dans les Paramètres n'est plus installée : ${name} sera utilisée.`,
    preferredLoading: "Lecture de la voix choisie dans les Paramètres…",
    preferredFailed: (name: string) =>
      `La voix choisie dans les Paramètres n'a pas pu être lue : ${name} sera utilisée.`,
    listen: "Écouter",
    stop: "Arrêter",
    listenTitle: "Écouter le titre du pack",
    stopTitle: "Arrêter la lecture du titre du pack",
    noTitle: "Saisissez un titre de pack pour l'écouter.",
    playing: "Lecture en cours…",
    preview: {
      tts_no_voice: "Aucune voix française n'est installée sur cet ordinateur.",
      tts_unavailable:
        "La synthèse vocale de Windows n'est pas disponible sur cet ordinateur.",
      tts_failed: "La voix n'a pas pu lire ce texte. Réessayez.",
    } as Record<string, string>,
    settings: {
      title: "Titres lus à voix haute",
      note: "Voix utilisée pour lire les titres des packs de podcasts (case « Lire les titres à voix haute » de l'éditeur de pack). Le débit est celui de Windows.",
      voice: "Voix",
      voiceOption: (name: string, language: string) => `${name} (${language})`,
      gone: "La voix choisie n'est plus installée : la voix par défaut sera utilisée.",
      goneOption: "Voix choisie (plus installée)",
      tryLabel: "Essayer la voix",
      trySample: "Bonjour ! Je lirai les titres de vos histoires.",
      failed: "La voix n'a pas pu être enregistrée.",
    },
    /** Pack terminé : avertissements des titres lus (voix de repli, titres muets…), dans le suivi de la tâche. */
    warnings: (shown: string[], more: number) =>
      more > 0
        ? `${shown.join(" ")} Et ${more} autre${more > 1 ? "s" : ""} avertissement${more > 1 ? "s" : ""}.`
        : shown.join(" "),
  },
  recovery: {
    title: (file: string) => `Le fichier ${file} était illisible`,
    /**
     * `file` : fichier restauré. `settings.json` porte les réglages, dont le dossier de cache
     * (revenu à l'emplacement par défaut) ; `sources.json` porte les sources.
     */
    body: (file: string, backup: string) =>
      `${
        file === "settings.json"
          ? "Les réglages sont revenus aux valeurs par défaut. Si vous aviez choisi un autre dossier de cache, vos histoires installées y sont toujours : choisissez-le de nouveau dans Paramètres pour les retrouver. "
          : file === "sources.json"
            ? "Les sources que vous aviez ajoutées ne sont plus dans la liste : seul le Telmi Store reste. "
            : "Il a été remplacé par les valeurs par défaut. "
      }L'ancien fichier est conservé sous le nom « ${backup} », à côté du fichier, dans le dossier de configuration.`,
    ack: "J'ai compris",
    ackFailed: "Impossible d'enregistrer l'acquittement.",
  },
  closeRetained: {
    title: "Écriture sur la Flam en cours",
    body: "La fenêtre se fermera à la fin de l'opération en cours. Les envois en attente ne partiront pas. Fermez à nouveau pour interrompre maintenant.",
    // Ce que fait la reprise de la Flam au prochain branchement, selon l'opération coupée.
    resume: {
      transfer:
        "La copie en cours sera nettoyée au prochain branchement de la Flam et l'histoire ne sera pas installée.",
      remove:
        "Le retrait sera repris au prochain branchement : une histoire à moitié retirée apparaît comme non listée et se retire de nouveau.",
      reorder:
        "La liste de la Flam gardera l'ancien ou le nouvel ordre, réparée au prochain branchement.",
    },
    keep: "Ne pas fermer",
  },
  sourcesScreen: {
    intro:
      "Les sources fournissent les histoires affichées dans la bibliothèque. Faites glisser une ligne ou utilisez les boutons Monter et Descendre pour changer l'ordre.",
    addCatalog: "Ajouter un catalogue",
    addFolder: "Ajouter un dossier",
    pickFolderTitle: "Choisir un dossier d'histoires",
    defaultFolderName: "Dossier d'histoires",
    listLabel: "Sources enregistrées",
    libraryNote: "Toujours en première position",
    moveUp: (name: string) => `Monter « ${name} »`,
    moveDown: (name: string) => `Descendre « ${name} »`,
    rename: (name: string) => `Renommer « ${name} »`,
    remove: (name: string) => `Supprimer « ${name} »`,
    moved: (name: string, position: number, total: number) =>
      `« ${name} » : position ${position} sur ${total}`,
    added: (name: string) => `Source « ${name} » ajoutée`,
    renamed: (name: string) => `Source renommée en « ${name} »`,
    removed: (name: string) => `Source « ${name} » supprimée`,
    catalogDialog: {
      title: "Ajouter un catalogue",
      description:
        "Un catalogue est une adresse web qui liste des histoires (format JSON).",
      name: "Nom",
      url: "Adresse du catalogue",
      urlHint: "Exemple : https://exemple.org/catalogue.json",
      submit: "Ajouter",
    },
    renameDialog: {
      title: "Renommer la source",
      name: "Nom",
      submit: "Renommer",
    },
    deleteDialog: {
      title: "Supprimer cette source ?",
      description: (name: string) =>
        `« ${name} » sera retirée de la liste. Aucun fichier de votre ordinateur n'est supprimé.`,
      confirm: "Supprimer",
    },
  },
  validation: {
    nameEmpty: "Le nom ne peut pas être vide.",
    nameControl: "Le nom ne peut pas contenir de caractères de contrôle.",
    nameTooLong: (max: number) => `Le nom ne peut pas dépasser ${max} caractères.`,
    urlEmpty: "L'adresse ne peut pas être vide.",
    urlTooLong: (max: number) => `L'adresse ne peut pas dépasser ${max} caractères.`,
    urlInvalid: "L'adresse doit commencer par http:// ou https:// et contenir un site.",
    urlCredentials:
      "L'adresse ne doit pas contenir d'identifiants (utilisateur:mot de passe@).",
    pathNotAbsolute: "Indiquez un chemin complet, par exemple C:\\Dossier.",
    pathNetwork: "Les chemins réseau ne sont pas acceptés.",
    pathNotExe: "Le fichier doit se terminer par .exe.",
  },
  podcasts: {
    title: "Podcasts",
    intro:
      "Ajoutez un flux de podcast, puis choisissez les épisodes à réunir dans un pack.",
    /** Mention d'usage de l'écran d'ajout : version courte décidée par le mainteneur (#220, spec § 4). */
    usageTitle: "Usage personnel",
    usage:
      "Usage personnel et familial uniquement : ne partagez pas les histoires créées. L'audio est repris tel quel, publicité comprise.",
    add: {
      title: "Ajouter un flux",
      url: "Adresse du flux (RSS)",
      urlHint: "Adresse complète, commençant par http:// ou https://.",
      name: "Nom (facultatif)",
      nameHint: "Par défaut, le titre du flux.",
      submit: "Ajouter le flux",
      adding: "Téléchargement du flux…",
      added: (name: string) => `Flux « ${name} » ajouté.`,
    },
    error: {
      invalidFeed: "Flux invalide",
      noAudio: "Aucun épisode audio",
      tooLarge: "Flux trop grand",
      network: "Flux injoignable",
      networkHint:
        "Vérifiez la connexion, puis réessayez. Un flux déjà ajouté reste consultable hors ligne.",
      blocked: "Adresse refusée",
      duplicate: "Flux déjà ajouté",
      invalidUrl: "Adresse invalide",
    },
    feeds: {
      title: "Mes flux",
      empty: {
        title: "Aucun flux podcast",
        description: "Ajoutez l'adresse d'un flux ci-dessus pour choisir ses épisodes.",
      },
      urlLabel: "Adresse :",
    },
    episodes: {
      title: (feed: string) => `Épisodes de « ${feed} »`,
      loading: "Chargement des épisodes…",
      refreshFailed:
        "Le flux n'a pas pu être actualisé : la version enregistrée est affichée.",
      offline: (date: string | null) =>
        date === null
          ? "Hors ligne : la version enregistrée du flux est affichée."
          : `Hors ligne : la version du flux du ${date} est affichée.`,
      empty: {
        title: "Aucun épisode",
        description: "Ce flux ne contient aucun épisode audio exploitable.",
      },
      list: "Épisodes",
      latest: (count: number) =>
        count === 1
          ? "Cocher le dernier épisode"
          : `Cocher les ${count} derniers épisodes`,
      latestDone: (
        outcome: {
          accepted: readonly unknown[];
          already: number;
          explicit: number;
          capped: number;
        },
        count: number,
      ) => {
        const checked = outcome.accepted.length;
        const done = `${checked} ${checked > 1 ? "épisodes cochés" : "épisode coché"}`;
        if (checked === count) return `Raccourci : ${done}.`;
        const causes = [
          outcome.already > 0
            ? `${outcome.already} déjà coché${outcome.already > 1 ? "s" : ""}`
            : null,
          outcome.explicit > 0
            ? `${outcome.explicit} explicite${outcome.explicit > 1 ? "s" : ""}`
            : null,
          outcome.capped > 0
            ? `${outcome.capped} refusé${outcome.capped > 1 ? "s" : ""} par les plafonds`
            : null,
        ].filter((cause) => cause !== null);
        const detail = causes.length > 0 ? ` (${causes.join(", ")})` : "";
        if (checked === 0) {
          return count === 1
            ? `Raccourci : le dernier épisode n'a pas été coché${detail}.`
            : `Raccourci : aucun des ${count} derniers épisodes n'a été coché${detail}.`;
        }
        return `Raccourci : ${done} sur les ${count} derniers${detail}.`;
      },
      explicit: "Explicite",
      unknownDate: "Date inconnue",
      unknownSize: "taille inconnue",
      size: (size: string, approximate: boolean) =>
        approximate ? `environ ${size}` : size,
      duration: (minutes: number) => `${minutes} min`,
      showMore: (remaining: number) => `Afficher plus (${remaining} restants)`,
      warnings: (count: number) =>
        count === 1 ? "1 élément du flux ignoré" : `${count} éléments du flux ignorés`,
    },
    /** Plafonds D1 en unités de la spec (1 Mo = 10⁶ octets, 1 Gio = 2³⁰ octets). */
    limits: { episode: "200 Mo", pack: "1 Gio" },
    refusal: {
      count: (max: number) => `Plafond atteint : ${max} épisodes par pack au plus.`,
      episodeTooBig: (size: string) =>
        `Épisode trop volumineux : ${size} au plus par épisode.`,
      total: (size: string) => `Plafond atteint : ${size} au plus par pack.`,
      duplicate: "Cet épisode est déjà dans la sélection.",
    },
    summary: {
      title: "Sélection",
      count: (count: number, max: number) =>
        `${count} ${count > 1 ? "épisodes sélectionnés" : "épisode sélectionné"} sur ${max} au plus`,
      size: (size: string, max: string) =>
        `taille estimée : environ ${size} sur ${max} au plus`,
      unknown: (count: number) =>
        count === 1
          ? "1 épisode de taille inconnue n'est pas compté"
          : `${count} épisodes de taille inconnue ne sont pas comptés`,
      clear: "Tout désélectionner",
      empty: "Aucun épisode sélectionné.",
    },
    pack: {
      title: "Composer un pack",
      link: "Composer le pack",
      back: "Retour aux épisodes",
      intro:
        "Rangez les épisodes choisis dans une arborescence libre, réglez le titre et la couverture : le pack deviendra une nouvelle histoire. Votre travail est enregistré automatiquement.",
      loading: "Chargement du brouillon…",
      loadFailed: "Impossible de lire le brouillon du pack.",
      draftSaving: "Enregistrement du brouillon…",
      draftSaved: "Brouillon enregistré.",
      draftFailed: "Le brouillon n'a pas pu être enregistré.",
      validateFailed: "La vérification du pack a échoué.",
      checking: "Vérification en cours…",
      unavailableBanner: (count: number) =>
        count === 1
          ? "1 épisode n'est plus disponible dans son flux : retirez-le du pack."
          : `${count} épisodes ne sont plus disponibles dans leur flux : retirez-les du pack.`,
      summary: {
        title: "Récapitulatif",
        episodes: (count: number, max: number) =>
          `${count} ${count > 1 ? "épisodes" : "épisode"} sur ${max} au plus`,
        levels: (levels: number, max: number) =>
          `${levels} ${levels > 1 ? "niveaux" : "niveau"} de menus sur ${max} au plus`,
        size: (size: string, max: string) =>
          `taille estimée : environ ${size} sur ${max} au plus`,
        unknown: (count: number) =>
          count === 1
            ? "1 épisode de taille inconnue n'est pas compté"
            : `${count} épisodes de taille inconnue ne sont pas comptés`,
      },
      limits: (options: number, levels: number) =>
        `Limites : ${levels} niveaux de menus, ${options} options par menu, aucune catégorie vide, chaque épisode une seule fois.`,
      tree: {
        title: "Arborescence",
        label: "Arborescence du pack",
        hint: (maxName: number) =>
          `L'ordre ci-dessous est l'ordre dans l'appareil. Utilisez les boutons pour monter, descendre ou déplacer un élément ; le glisser-déposer par la poignée est aussi possible à la souris. Un nom de catégorie compte ${maxName} caractères au plus.`,
        empty: {
          title: "Le pack est vide",
          description:
            "Choisissez des épisodes dans l'écran Podcasts : ils apparaîtront ici.",
        },
        root: "Racine",
        untitled: "(sans nom)",
        menuEmpty: "Aucun élément.",
        episode: "Épisode",
        category: "Catégorie",
        unavailableBadge: "Indisponible",
        unknownTitle: "Épisode indisponible",
        categoryName: (position: string) => `Nom de la catégorie n° ${position}`,
        newCategory: "Nouvelle catégorie",
        addCategory: "Ajouter une catégorie à la racine",
        addCategoryIn: (menu: string) => `Ajouter une catégorie dans « ${menu} »`,
        up: (label: string) => `Monter « ${label} »`,
        down: (label: string) => `Descendre « ${label} »`,
        moveTo: (label: string) => `Déplacer « ${label} » vers`,
        movePlaceholder: "Choisir un menu…",
        moveButton: "Déplacer",
        moveConfirm: (label: string) => `Déplacer « ${label} » vers le menu choisi`,
        loadingTitle: "Chargement du titre…",
        remove: (label: string) => `Retirer « ${label} »`,
        removeCategory: (label: string) => `Supprimer la catégorie « ${label} »`,
      },
      announce: {
        moved: (label: string, position: number, count: number) =>
          `« ${label} » : position ${position} sur ${count}.`,
        movedTo: (label: string, menu: string) =>
          `« ${label} » déplacé vers « ${menu} ».`,
        removed: (label: string) => `« ${label} » retiré du pack.`,
        added: (label: string, menu: string) => `« ${label} » ajouté à « ${menu} ».`,
        refused: "Déplacement impossible : limite de l'arborescence atteinte.",
        categoryImage: (category: string, chosen: boolean) =>
          chosen
            ? `Image de la catégorie « ${category} » : épisode choisi.`
            : `Image de la catégorie « ${category} » : automatique.`,
      },
      pool: {
        title: "Épisodes sélectionnés, pas encore placés",
        addRoot: (label: string) => `Ajouter « ${label} » à la racine`,
        addAll: "Tout ajouter à la racine",
      },
      metadata: {
        title: "Histoire",
        storyTitle: "Titre",
        titleHint: (feedTitle: string | null, multiple: boolean) =>
          multiple
            ? "Obligatoire : le pack réunit plusieurs flux."
            : feedTitle === null
              ? "Facultatif tant que le pack n'utilise qu'un seul flux."
              : `Laissez vide pour reprendre le titre du flux : « ${feedTitle} ».`,
        subtitle: "Sous-titre",
        subtitleHint: (prefilledFromFeed: boolean) =>
          !prefilledFromFeed
            ? "Facultatif."
            : "Prérempli avec le copyright du flux : modifiez-le, ou effacez-le pour n'avoir aucun sous-titre.",
        age: "Âge conseillé",
        ageOption: (age: number) => (age === 0 ? "Tout public" : `${age} ans`),
        description: "Description",
        descriptionHint:
          "Laissez vide pour utiliser la liste des épisodes (regroupée par flux si le pack en réunit plusieurs).",
      },
      cover: {
        title: "Couverture",
        label: "Image de la couverture",
        hint: "Par défaut, l'image du podcast du premier flux utilisé.",
        default: "Image du premier flux (par défaut)",
        feed: (name: string) => `Image du flux « ${name} »`,
        episode: (title: string) => `Image de l'épisode « ${title} »`,
        missing: "Image indisponible : choisissez-en une autre",
      },
      categoryImage: {
        legend: (category: string) => `Image de la catégorie « ${category} »`,
        auto: "Automatique",
        autoHint: "Image du premier épisode de la catégorie.",
        /** Titre porté par plusieurs épisodes : le rang les distingue. */
        ranked: (title: string, rank: number) => `${title} (n° ${rank})`,
      },
      problems: {
        title: "À corriger avant de créer l'histoire",
        ok: "Le pack est prêt à être créé.",
        at: (place: string) => `(${place})`,
        noEpisode: "Le pack ne contient aucun épisode.",
        tooManyEpisodes: (max: number) => `Trop d'épisodes : ${max} au plus par pack.`,
        tooDeep: (max: number) => `Arborescence trop profonde : ${max} niveaux au plus.`,
        tooManyOptions: (max: number) => `Trop d'options dans un menu : ${max} au plus.`,
        emptyCategory: "Une catégorie est vide : ajoutez-y un épisode ou supprimez-la.",
        invalidCategoryName: (max: number) =>
          `Le nom d'une catégorie doit faire de 1 à ${max} caractères.`,
        duplicateEpisode: "Un épisode est présent deux fois dans le pack.",
        invalidId: "Un épisode du brouillon est invalide : retirez-le.",
        titleRequired: "Le titre est obligatoire quand le pack réunit plusieurs flux.",
        textTooLong: "Un texte est trop long.",
        episodeUnavailable: "Un épisode n'est plus disponible : retirez-le du pack.",
        episodeTooLarge: (max: string) =>
          `Un épisode dépasse la taille maximale de ${max}.`,
        packTooLarge: (max: string) => `Le pack dépasse la taille maximale de ${max}.`,
        coverUnavailable: "La couverture choisie n'est plus disponible.",
      },
      create: {
        label: "Créer l'histoire",
        creating: "Création en cours…",
        blocked:
          "Corrigez les points signalés ci-dessus : la création est possible dès que le pack est prêt.",
        checking: "Vérification du pack en cours…",
        checkFailed:
          "La vérification du pack a échoué : modifiez le pack pour la relancer.",
        inProgress:
          "La création de ce pack est déjà en cours : suivez-la dans le panneau des tâches, ou modifiez le pack pour en créer un autre.",
        refused: "Le pack n'a pas été créé : corrigez les points suivants.",
        started: (title: string) =>
          `La création de « ${title} » a démarré : suivez-la dans le panneau des tâches.`,
        compositionSaved: (name: string, isNew: boolean) =>
          isNew
            ? `Composition enregistrée sous « ${name} ».`
            : `Composition « ${name} » réutilisée.`,
        compositionNotSaved: (reason: string) =>
          `La composition n'a pas été enregistrée : ${reason}.`,
      },
      rights: {
        title: "Avant de créer votre premier pack",
        text: "Les podcasts appartiennent à leurs auteurs. Les histoires créées ici sont réservées à un usage personnel, dans le cadre familial : ne les partagez pas et ne les redistribuez pas. L'audio est téléchargé tel que l'éditeur le diffuse, publicité comprise.",
        accept: "J'ai compris",
        cancel: "Annuler",
      },
    },
    /** Compositions nommées réutilisables (#461, spec § 14). */
    compositions: {
      title: "Compositions enregistrées",
      intro:
        "Chaque pack créé garde sa composition. Rouvrez-la pour refaire le pack ou en dériver un autre : chaque création donne une nouvelle histoire.",
      loading: "Chargement des compositions…",
      loadFailed: "Impossible de lire les compositions enregistrées.",
      empty:
        "Aucune composition enregistrée : elles apparaissent ici après la création d'un pack.",
      count: (count: number, max: number) =>
        count >= max
          ? `${count} compositions sur ${max} au plus : supprimez-en une pour conserver les suivantes.`
          : `${count} ${count > 1 ? "compositions" : "composition"} sur ${max} au plus.`,
      episodes: (count: number) => `${count} ${count > 1 ? "épisodes" : "épisode"}`,
      lastUsed: (date: string) => `dernier pack le ${date}`,
      missingFeeds: (count: number) =>
        count === 1
          ? "1 flux supprimé : ses épisodes sont indisponibles."
          : `${count} flux supprimés : leurs épisodes sont indisponibles.`,
      open: "Ouvrir",
      openLabel: (name: string) => `Ouvrir « ${name} » dans l'éditeur`,
      opening: (name: string) => `Ouverture de « ${name} »…`,
      rename: (name: string) => `Renommer « ${name} »`,
      remove: (name: string) => `Supprimer « ${name} »`,
      renamed: (name: string) => `Composition renommée en « ${name} ».`,
      removed: (name: string) => `Composition « ${name} » supprimée.`,
      replaceDialog: {
        title: "Remplacer le brouillon ?",
        description: (name: string) =>
          `Le brouillon en cours sera remplacé par « ${name} ».`,
        confirm: "Remplacer",
      },
      renameDialog: {
        title: "Renommer la composition",
        name: "Nom",
        submit: "Renommer",
      },
      deleteDialog: {
        title: "Supprimer cette composition ?",
        description: (name: string) =>
          `« ${name} » sera retirée de la liste. Les histoires déjà créées ne sont pas touchées.`,
        confirm: "Supprimer",
      },
      nameTooLong: (max: number) => `Le nom ne peut pas dépasser ${max} caractères.`,
    },
  },
  settingsScreen: {
    paths: "Programmes et dossiers",
    save: "Enregistrer",
    saved: "Paramètres enregistrés",
    automatic: "Automatique",
    browse: "Parcourir…",
    autoPlaceholder: "Automatique",
    issuesTitle: "Points à vérifier",
    player: "Chemin de flam-player",
    cacheDir: "Dossier de cache",
    pickPlayer: "Choisir flam-player.exe",
    pickCache: "Choisir le dossier de cache",
    used: (path: string) => `Utilisé : ${path}`,
    notFound: "Aucun fichier trouvé : choisissez-le avec « Parcourir… ».",
    cache: {
      title: "Cache",
      usage: (size: string, path: string) => `${size} occupés dans ${path}`,
      empty: (path: string) => `Le cache est vide (${path}).`,
      /** Part du cache occupée par les MP3 de podcasts (déjà comprise dans la taille ci-dessus). */
      mp3: (size: string) =>
        `Dont MP3 de podcasts : ${size}, gardés pour refaire un pack sans tout retélécharger.`,
      mp3Empty: "Aucun MP3 de podcast n'est gardé en cache.",
      clear: "Vider le cache",
      /** Annoncé pendant le vidage (le bouton est verrouillé, #578). */
      clearing: "Vidage du cache en cours…",
      confirmTitle: "Vider le cache ?",
      /**
       * `dirs` : sous-dossiers marqués que le vidage videra (`cache_usage.subdirs`). La phrase
       * sur les histoires installées n'apparaît que si `library` en fait partie (#591).
       */
      confirmDescription: (dirs: readonly string[], path: string, size: string) =>
        dirs.length === 0
          ? `Aucun dossier créé par Flam Launcher n'est à vider dans « ${path} ». Rien ne sera vidé.`
          : `${
              dirs.includes("library")
                ? "Les histoires installées seront retirées de ce PC (leur dossier est vidé) : il faudra les réinstaller pour les jouer ou les envoyer sur la Flam. "
                : ""
            }Seuls les dossiers créés par Flam Launcher (${dirs.join(", ")}) seront vidés dans « ${path} ». Le reste de ce dossier n'est pas touché. Espace libéré : ${size}.`,
      /** Relevé du cache illisible à l'ouverture de la confirmation : le pire est annoncé (#591). */
      confirmDescriptionUnknown: (path: string) =>
        `Les histoires installées seront retirées de ce PC (leur dossier est vidé) : il faudra les réinstaller pour les jouer ou les envoyer sur la Flam. Seuls les dossiers créés par Flam Launcher seront vidés dans « ${path} ». Le reste de ce dossier n'est pas touché.`,
      cleared: (size: string) => `Cache vidé : ${size} libérés`,
    },
    appearance: "Apparence",
    /** Envoi sur la Flam (#609). */
    send: {
      title: "Envoi sur la Flam",
      removeAfterSend: "Retirer de Mes histoires après l'envoi sur la Flam",
      removeAfterSendHint:
        "Une fois l'histoire envoyée et son écriture confirmée sur la Flam, sa copie est retirée de ce PC pour libérer de la place. L'histoire reste sur la Flam ; pour la renvoyer plus tard, il faudra la réinstaller. La progression de l'enfant est conservée. Avant de débrancher la Flam, utilisez « Éjecter la Flam ».",
      failed: "Ce réglage n'a pas pu être enregistré.",
    },
    playback: {
      title: "Lecture",
      minimize: "Réduire la fenêtre pendant la lecture",
      minimizeHint:
        "Quand flam-player s'ouvre dans une fenêtre séparée, la fenêtre du launcher est réduite à son lancement, puis rétablie à la fin.",
    },
    themeFailed: "Le thème n'a pas pu être enregistré.",
    thumbFit: {
      title: "Vignettes",
      label: "Vignettes : remplissage",
      note: "Les vignettes de bibliothèque des histoires converties sont en 104 × 136 (portrait). Ce réglage s'applique aux prochaines conversions ; les histoires déjà converties gardent leur vignette.",
      failed: "Le remplissage des vignettes n'a pas pu être enregistré.",
      options: {
        flou: {
          label: "Flou",
          hint: "Image entière, fond rempli par la même image agrandie et floutée.",
        },
        rogner: {
          label: "Rogner",
          hint: "Image agrandie et rognée au centre pour remplir la vignette.",
        },
        bandes: {
          label: "Bandes",
          hint: "Image entière, bandes transparentes en haut et en bas.",
        },
      },
    },
    about: {
      title: "À propos",
      version: (name: string, version: string) => `${name} — version ${version}`,
      license: "Licence GPL v3.",
    },
  },
  welcome: {
    title: "Bienvenue",
    version: (name: string, version: string) => `${name} — version ${version}`,
    error: "Impossible de lire les informations de l'application.",
  },
  library: {
    sourceNotFound: {
      title: "Source introuvable",
      description: "Cette source n'existe plus. Choisissez-en une dans la liste.",
      action: "Aller à la première source",
    },
    refresh: "Actualiser",
    refreshing: "Actualisation…",
    loading: "Chargement des histoires…",
    loadFailed: "Impossible de charger les histoires de cette source.",
    refreshed: (name: string) => `« ${name} » est à jour`,
    refreshedOffline: (name: string) =>
      `« ${name} » est injoignable : les données enregistrées sont affichées.`,
    offlineTitle: "Hors ligne",
    offlineBody: (when: string | null) =>
      when
        ? `Le catalogue est injoignable. Les histoires affichées viennent de la dernière vérification, le ${when}.`
        : "Le catalogue est injoignable. Les histoires affichées viennent des données enregistrées.",
    problemTitle: "Le dernier rafraîchissement a échoué",
    warningsTitle: (n: number) =>
      n <= 1 ? `${n} élément ignoré` : `${n} éléments ignorés`,
    warningsHint:
      "Ces éléments ont été ignorés (illisibles, invalides ou dans un format non reconnu). Formats acceptés : histoires Telmi (.zip), STUdio (.zip) et Flam (.plain, .plain.pk). Les histoires STUdio s'installent, mais elles ne s'envoient que sur une Flam en firmware 2.x et ne se lisent pas sur l'ordinateur.",
    filtersLabel: "Recherche et filtres",
    search: "Rechercher une histoire",
    searchPlaceholder: "Titre, auteur, catégorie…",
    age: "Âge de l'enfant",
    allAges: "Tous les âges",
    ageOption: (age: number) => `${age} ans`,
    category: "Catégorie",
    allCategories: "Toutes les catégories",
    sort: "Trier par",
    sortOptions: {
      title: "Titre (A-Z)",
      recent: "Plus récentes",
      popular: "Popularité",
    },
    resultCount: (shown: number, total: number) =>
      shown === total ? storyCount(total) : `${storyCount(shown)} sur ${total}`,
    gridLabel: (name: string) => `Histoires de ${name}`,
    showMore: (remaining: number) => `Afficher plus (${remaining} restantes)`,
    resetFilters: "Réinitialiser les filtres",
    empty: {
      library: {
        title: "Aucune histoire installée",
        description:
          "Les histoires installées apparaîtront ici. Parcourez un catalogue ou un dossier pour en ajouter.",
        action: "Gérer les sources",
      },
      catalog: {
        title: "Ce catalogue est vide",
        description: "Il ne propose aucune histoire pour le moment.",
      },
      folder: {
        title: "Aucune histoire dans ce dossier",
        description:
          "Ajoutez des archives .zip, des dossiers .plain ou des fichiers .plain.pk dans le dossier.",
      },
      podcast: {
        title: "Flux podcast",
        description: "La sélection des épisodes de ce flux n'est pas encore disponible.",
      },
      noResults: {
        title: "Aucun résultat",
        description: "Aucune histoire ne correspond à la recherche et aux filtres.",
      },
    },
    state: {
      to_download: "À télécharger",
      to_convert: "À convertir",
      to_install: "À installer",
      studio: "Format STUdio",
      installed: "Installée",
      update_available: "Mise à jour disponible",
      in_progress: "En cours",
      incompatible: "Incompatible",
      untested: "Non testée",
    },
  },
  compat: {
    level: {
      compatible: "Compatible",
      warnings: "Avertissements",
      incompatible: "Incompatible",
      untested: "Non testée",
    },
    /** Raison affichée quand la fiche n'en fournit pas. */
    defaultReason: {
      compatible: "Aucun problème constaté lors des essais.",
      warnings: "Jouable, mais des limites ont été constatées lors des essais.",
      incompatible: "Cette histoire a échoué lors des essais de compatibilité.",
      untested: "Pas encore évaluée.",
    },
    /** Raison propre à chaque statut d'échec connu de la campagne. */
    failureReason: {
      ECHEC_TELECHARGEMENT: "Le téléchargement a échoué lors des essais.",
      ECHEC_CONVERSION: "La conversion a échoué lors des essais.",
      ECHEC_VALIDATION: "L'histoire convertie n'est pas valide.",
      ECHEC_EXECUTION: "L'histoire s'arrête ou se bloque lors de son exécution.",
    } as Record<string, string>,
    stale: "Résultat périmé : évalué avec une autre version du convertisseur.",
    issue: {
      title: "Rapport de compatibilité indisponible",
      unreadable:
        "Le fichier de statuts est illisible : les histoires apparaissent non testées.",
      tooLarge: "Le fichier de statuts est trop volumineux : il n'est pas lu.",
      unsupportedVersion: (found: number | null, supported: number) =>
        `Le fichier de statuts utilise un format ${found === null ? "inconnu" : `n° ${found}`} (pris en charge : n° ${supported}) : mettez à jour l'application.`,
    },
    install: {
      title: (story: string) => `Installer « ${story} » malgré tout ?`,
      description: "Cette histoire est signalée comme incompatible.",
      body: "Elle risque de ne pas se convertir ou de ne pas fonctionner correctement dans le player. Vous pouvez l'installer quand même.",
      confirm: "Installer quand même",
      cancel: "Annuler",
    },
  },
  common: {
    close: "Fermer",
    cancel: "Annuler",
    loading: "Chargement…",
    /** Nom du filtre du sélecteur de fichier exécutable. */
    executableFilter: "Exécutable (.exe)",
  },
  sources: {
    kind: {
      library: "Bibliothèque",
      catalog: "Catalogue",
      folder: "Dossier local",
      podcast: "Podcast",
    },
    title: "Sources",
    add: "Ajouter une source",
    count: storyCount,
    status: {
      offline: "Hors ligne",
      error: "Erreur",
    },
  },
  story: {
    age: (min: number, max?: number) =>
      max === undefined ? `Dès ${min} ans` : `${min}-${max} ans`,
    progress: (title: string) => `Progression : ${title}`,
  },
  status: {
    installed: "Installée",
    available: "Disponible",
    compatible: "Compatible",
    partial: "Compatibilité partielle",
    incompatible: "Incompatible",
    new: "Nouveau",
  },
  tasks: {
    title: "Tâches en cours",
    empty: "Aucune tâche en cours.",
    kind: {
      download: "Téléchargement",
      convert: "Conversion",
      install: "Installation",
    },
    state: {
      queued: "En attente",
      running: "En cours",
      done: "Terminée",
      error: "Échec",
      cancelled: "Annulée",
    },
    cancel: (title: string) => `Annuler : ${title}`,
    retry: (title: string) => `Réessayer : ${title}`,
    dismiss: (title: string) => `Retirer : ${title}`,
    progress: (title: string) => `Progression : ${title}`,
    /** Annonce d'un changement d'état, lue par les lecteurs d'écran. */
    announce: (title: string, kind: string, state: string, detail?: string) =>
      `${kind} « ${title} » : ${state}${detail ? `. ${detail}` : ""}`,
  },
  stories: {
    detail: {
      cover: (title: string) => `Vignette de « ${title} »`,
      noDescription: "Aucune description.",
      facts: {
        age: "Âge",
        category: "Catégorie",
        author: "Auteur",
        voice: "Voix",
        license: "Licence",
        size: "Taille",
        source: "Source",
      },
      badges: "Statut de l'histoire",
      unknown: "Non précisé",
    },
    actions: {
      downloadAndPlay: "Télécharger et jouer",
      download: "Télécharger",
      installAndPlay: "Installer et jouer",
      /** Installation suivie de l'envoi sur la Flam branchée (#609). */
      installAndSend: "Installer et envoyer",
      downloadAndSend: "Télécharger et envoyer",
      /** Info-bulle et texte accessible de « Installer et envoyer ». */
      installAndSendHelp:
        "Installe l'histoire dans Mes histoires, puis l'envoie sur la Flam branchée. Si la Flam est débranchée ou remplacée pendant l'installation, l'histoire reste installée et rien n'est envoyé.",
      /** Fiche d'une histoire en cours d'installation par « Installer et envoyer ». */
      sendPlanned: "Elle sera envoyée sur la Flam à la fin de l'installation.",
      sendPlanUnwatched:
        "À la fin de l'installation, son envoi sur la Flam sera à confirmer dans le panneau d'envoi.",
      sendPlanLost:
        "La Flam a été débranchée ou a changé : l'histoire sera installée, mais pas envoyée.",
      install: "Installer",
      play: "Jouer",
      playFromFolder: "Jouer depuis le dossier",
      update: "Mettre à jour",
      /** Info-bulle et texte accessible du badge « Mise à jour disponible » et du bouton. */
      updateHelp:
        "Le fichier de cette histoire a changé dans sa source depuis l'installation. « Mettre à jour » recopie la nouvelle version dans la bibliothèque ; le cas échéant, renvoyez-la ensuite sur la Flam.",
      /** Raison du bouton « Jouer » désactivé d'une histoire STUdio installée (Q1). */
      playUnavailable: "Lecture sur le PC indisponible pour une histoire STUdio",
      remove: "Supprimer",
      removing: "Suppression en cours.",
      removePlaying: "Suppression impossible : cette histoire est en cours de lecture.",
      cancel: "Annuler",
      /** Libellé accessible des actions rapides d'une carte. */
      forStory: (action: string, title: string) => `${action} « ${title} »`,
    },
    progress: {
      queued: "En attente",
      downloading: "Téléchargement",
      downloadingSize: (received: string, total: string) =>
        `Téléchargement : ${received} sur ${total}`,
      converting: "Conversion",
      installing: "Installation",
      phase: {
        images: "images",
        audio: "audio",
        scripts: "scripts",
        zip: "archive",
      } as Record<string, string>,
      convertingPhase: (phase: string) => `Conversion : ${phase}`,
      preparing: "Préparation du pack",
      preparingPhase: (phase: string) => `Préparation du pack : ${phase}`,
      preparePhase: {
        validation: "vérification des épisodes",
        speech: "lecture des titres à voix haute",
        download: "téléchargement des épisodes",
        audio: "conversion audio",
        images: "pochettes",
        archive: "assemblage",
      } as Record<string, string>,
      prepared: "Pack préparé, en attente de conversion",
      packDropped: (titles: string[]) =>
        `${titles.length} épisode${titles.length > 1 ? "s" : ""} non inclus : ${titles.join(", ")}`,
    },
    playing: "En cours de lecture dans flam-player",
    studioNotice:
      "Format STUdio : cette histoire s'installe, puis s'envoie uniquement sur une Flam en firmware 2.x (version visible dans « Ma Flam »). Elle ne se lit pas sur l'ordinateur.",
    /** Badge de format d'une histoire STUdio installée (pack Lunii). */
    luniiFormat: "STUdio",
    formatReplace: {
      title: (story: string) => `Remplacer « ${story} » ?`,
      description: (installed: "flam" | "lunii") =>
        installed === "flam"
          ? "Cette histoire est déjà dans « Mes histoires » au format Flam (histoire Telmi)."
          : "Cette histoire est déjà dans « Mes histoires » au format STUdio.",
      toLunii:
        "La version STUdio remplacera la version Telmi : « Mes histoires » n'en garde qu'une. Elle ne se lit pas sur le PC et ne s'envoie que sur une Flam en firmware 2.x. La progression de l'enfant est conservée.",
      toFlam:
        "La version Telmi remplacera la version STUdio : « Mes histoires » n'en garde qu'une. La progression de l'enfant est conservée.",
      confirm: "Remplacer",
      cancel: "Annuler",
    },
    playerBusy:
      "Une histoire est déjà en cours de lecture : fermez flam-player pour jouer.",
    removeDialog: {
      title: (title: string) => `Supprimer « ${title} » ?`,
      description:
        "L'histoire est retirée de « Mes histoires » et son fichier converti est supprimé. Si elle est sur la Flam, elle y reste. Vous pourrez la réinstaller depuis sa source.",
      confirm: "Supprimer",
      purgeSaves: "Supprimer aussi la progression",
      purgeSavesHint:
        "Efface les sauvegardes de l'enfant. Cette action est irréversible. Décochée, la progression est conservée et retrouvée si l'histoire est réinstallée.",
    },
    toast: {
      entryGone: (title: string) =>
        `« ${title} » n'est plus proposée par sa source (catalogue ou dossier modifié). Rafraîchissez la source, puis relancez l'installation depuis la fiche.`,
      savesLost: (title: string, detail: string) =>
        `La progression de « ${title} » n'a pas pu être conservée telle quelle : ${detail}`,
      installed: (title: string) => `« ${title} » est installée`,
      installedWarnings: (title: string, count: number) =>
        `« ${title} » est installée (${count <= 1 ? `${count} avertissement` : `${count} avertissements`} à la conversion)`,
      removed: (title: string) => `« ${title} » a été supprimée`,
      finished: (title: string) => `Lecture de « ${title} » terminée`,
      playerFailed: (title: string, detail: string) =>
        `flam-player s'est arrêté sur une erreur pendant « ${title} ».${detail ? ` ${detail}` : ""}`,
    },
  },
  deviceScreen: {
    intro:
      "Les histoires installées sur votre Flam. Vous pouvez en envoyer depuis « Mes histoires » ou en retirer ; la Flam n'est modifiée que sur votre demande.",
    searching: "Recherche de la Flam…",
    notDetected: {
      title: "Aucune Flam détectée",
      description: "Activez le mode connexion USB sur la Flam",
      retry: "Réessayer",
    },
    announce: {
      detected: "Flam détectée",
      lost: "Flam débranchée ou mode connexion USB désactivé",
      stories: (count: number) => `${storyCount(count)} sur la Flam`,
    },
    detected: {
      title: "Flam détectée",
      volume: "Volume",
      fileSystem: "Système de fichiers",
      unnamedVolume: "(sans nom)",
      firmware: "Firmware",
      firmwareUnreadable: "illisible",
      several: (count: number) =>
        `${count} Flam sont branchées. Seule la première est affichée ; débranchez les autres.`,
    },
    space: {
      label: "Espace utilisé sur la Flam",
      summary: (free: string, total: string) => `${free} libres sur ${total}`,
      unavailable: "Espace disque indisponible",
    },
    warnings: {
      title: "À noter",
      missingStr: "Le dossier des histoires (str) est introuvable sur la Flam.",
      missingLibrary: "Le dossier de l'index (etc/library) est introuvable sur la Flam.",
      unexpectedMdf: (size: number) =>
        `Le fichier .mdf de la Flam a une taille inattendue (${size} octets).`,
      mdfTruncated: "Le fichier .mdf de la Flam est tronqué ou vide.",
      mdfTooLarge: "Le fichier .mdf de la Flam est démesuré : il n'a pas été lu.",
      backupFailed:
        "La copie de sauvegarde du fichier .mdf n'a pas pu être enregistrée sur cet ordinateur.",
    },
    stories: {
      title: (count: number) => `Histoires sur la Flam (${count})`,
      listLabel: "Histoires installées sur la Flam",
      loading: "Lecture des histoires…",
      refresh: "Actualiser la liste",
      untitled: (shortId: string) => `Histoire ${shortId}`,
      idDetail: (shortId: string) => `Identifiant ${shortId}`,
      hidden: "Masquée",
      lunii: "Lunii",
      unlisted: "Non listée",
      missing: "Dossier manquant",
      emptyTitle: "Aucune histoire sur la Flam",
      emptyDescription: "Les histoires envoyées depuis le launcher apparaîtront ici.",
      keysProblemTitle: "Titres des histoires illisibles",
      keysProblem: (detail: string) =>
        `Les clés du fichier .mdf n'ont pas pu être lues (${detail}). Les histoires sont nommées d'après la bibliothèque ou le catalogue, sinon par leur identifiant.`,
      loadFailed: "Impossible de lire les histoires de la Flam.",
      noAnswer: "La Flam n'a pas répondu : vérifiez la connexion USB.",
    },
    index: {
      visibleAbsent: "La Flam n'a pas d'index d'histoires (etc/library/list).",
      visibleDamaged:
        "L'index des histoires de la Flam contient des lignes invalides ou en double : elles sont ignorées.",
      visibleUnreadable:
        "L'index des histoires de la Flam est illisible. Les dossiers trouvés sont affichés comme non listés.",
      hiddenDamaged:
        "L'index des histoires masquées contient des lignes invalides ou en double : elles sont ignorées.",
      hiddenUnreadable: "L'index des histoires masquées est illisible.",
      truncated: (limit: number) =>
        `La liste est tronquée : seules les ${limit} premières histoires sont affichées.`,
      strUnreadable:
        "Le dossier des histoires (str) est illisible : seules les histoires de l'index sont affichées.",
    },
    order: {
      edit: "Modifier l'ordre",
      busy: "Ordre non modifiable pendant un envoi.",
      firmware:
        "Ordre non modifiable : écriture sur cette Flam bloquée (voir l'état du firmware).",
      title: "Ordre des histoires sur la Flam",
      help: "Glissez les histoires, ou utilisez les boutons Tout en haut, Monter, Descendre et Tout en bas (touches Début et Fin au clavier), puis enregistrez. La Flam affiche les histoires dans cet ordre.",
      othersNote:
        "Les histoires masquées ou non listées ne font pas partie de l'ordre et ne bougent pas.",
      listLabel: "Ordre des histoires visibles",
      position: (position: number) => `${position}.`,
      moveTop: (name: string) => `Tout en haut « ${name} »`,
      moveUp: (name: string) => `Monter « ${name} »`,
      moveDown: (name: string) => `Descendre « ${name} »`,
      moveBottom: (name: string) => `Tout en bas « ${name} »`,
      moved: (name: string, position: number, total: number) =>
        `« ${name} » déplacée en position ${position} sur ${total}`,
      save: "Enregistrer l'ordre",
      saving: "Enregistrement de l'ordre…",
      cancel: "Annuler",
      unchangedHint: "Déplacez au moins une histoire pour enregistrer un nouvel ordre.",
      reset: "La liste de la Flam a changé : l'ordre a été réinitialisé.",
      saved:
        "Ordre enregistré sur la Flam. Redémarrez la Flam pour qu'elle affiche le nouvel ordre.",
      unchanged: "L'ordre de la Flam était déjà celui-ci : rien n'a été écrit.",
      failed: (message: string) => `Impossible d'enregistrer l'ordre : ${message}`,
    },
  },
  deviceTransfer: {
    send: "Envoyer sur la Flam",
    replace: "Remplacer sur la Flam",
    /** La copie de bibliothèque est plus ancienne que sa source dossier (#288). */
    staleCopy:
      "Source modifiée depuis l'installation : mettez l'histoire à jour avant de l'envoyer, sinon l'ancienne version partira",
    forStory: (action: string, title: string) => `${action} « ${title} »`,
    reasons: {
      noDevice: "Flam non détectée",
      present: "Déjà sur la Flam",
      noSpace: "Espace insuffisant sur la Flam",
      firmware: "Écriture bloquée sur cette Flam (voir « Ma Flam »)",
      luniiNeedsV2:
        "Les histoires STUdio demandent une Flam en firmware 2.x. Cette Flam est en 1.x : elle ne peut pas les lire.",
      queued: "Dans la file d'envoi",
      checking: "Lecture de la Flam…",
      removing: "Retrait en cours sur la Flam",
    },
    experimental: {
      title: "Fonction expérimentale, non officielle",
      description:
        "L'envoi d'histoires sur la Flam n'est ni prévu ni garanti par son fabricant.",
      body: "Le launcher écrit directement sur la mémoire de la Flam. Il vérifie chaque copie avant de l'installer et ne touche à rien en cas d'échec, mais gardez une copie de vos histoires. Ne débranchez pas la Flam pendant un envoi.",
      confirm: "J'ai compris, envoyer",
      /** Même avertissement, ouvert par « Installer et envoyer » : l'installation vient d'abord. */
      confirmInstall: "J'ai compris, installer et envoyer",
    },
    panel: {
      title: "Envoi sur la Flam",
      overall: (done: number, total: number) => `Histoire ${done} sur ${total}`,
      overallLabel: "Progression de l'envoi",
      current: (title: string) => `En cours : ${title}`,
      currentLabel: (title: string) => `Progression de « ${title} »`,
      phase: { copy: "Copie sur la Flam", verify: "Vérification de la copie" },
      estimate: (minutes: number) =>
        minutes <= 1
          ? "Environ 1 minute restante"
          : `Environ ${minutes} minutes restantes`,
      cancel: "Annuler l'envoi",
      cancelling: "Annulation en cours…",
      removeFromQueue: (title: string) => `Retirer « ${title} » de la file`,
      /** Envoi réussi, mais la Flam a changé juste après (#577, `device_changed`). */
      deviceChangedAfterSend:
        "La Flam a été débranchée, remplacée ou momentanément illisible juste après cet envoi : les histoires suivantes ne sont pas envoyées. Rebranchez la Flam d'origine pour vérifier.",
      recoveryFailures: (failures: string[]) =>
        `Attention : un transfert interrompu n'a pas pu être repris sur la Flam (${failures.join(" ; ")}). Fermez ce qui utilise la Flam, puis débranchez-la et rebranchez-la.`,
      dismiss: "Fermer la liste",
      listLabel: "File d'envoi",
      state: {
        waiting: "En attente",
        sending: "Envoi en cours",
        done: "Envoyée",
        failed: "Échec",
        cancelled: "Annulée",
      },
      announce: {
        started: (title: string) => `Envoi de « ${title} » commencé`,
        done: (title: string) => `« ${title} » envoyée sur la Flam`,
        failed: (title: string, message: string) =>
          `Échec de l'envoi de « ${title} » : ${message}`,
        cancelled: "Envoi annulé",
        finished: "Envoi terminé",
        removedFromPc: (title: string) =>
          `« ${title} » retirée de Mes histoires : elle reste sur la Flam`,
        removeFromPcFailed: (title: string, message: string) =>
          `« ${title} » n'a pas pu être retirée du PC : ${message}`,
        keptUnconfirmed: (title: string) =>
          `« ${title} » est bien sur la Flam. Sa copie est gardée sur ce PC par prudence : vous pouvez la retirer avec « Retirer du PC », puis éjectez la Flam avec « Éjecter la Flam » avant de la débrancher.`,
      },
      /** Après un envoi réussi : retrait de la copie gardée dans « Mes histoires » (#609). */
      pcCopy: {
        remove: "Retirer du PC",
        /** Nom accessible : contient le libellé visible, distinct de « Retirer … de la file ». */
        removeLabel: (title: string) => `Retirer du PC : « ${title} »`,
        removing: "Retrait du PC…",
        hint: "L'histoire reste sur la Flam. Pour la renvoyer plus tard, il faudra la réinstaller. Avant de débrancher la Flam, utilisez « Éjecter la Flam ».",
        removed:
          "Retirée de Mes histoires : l'histoire reste sur la Flam. Pour la renvoyer plus tard, il faudra la réinstaller.",
        failed: (message: string) =>
          `L'histoire est envoyée, mais n'a pas pu être retirée du PC : ${message}`,
        /** « Retirer du PC » refusé : le bouton n'est plus proposé. */
        refused: (message: string) => `L'histoire n'a pas été retirée du PC : ${message}`,
        /** Écriture non confirmée sur le support (réglage actif ou non). */
        unconfirmed:
          "L'histoire est bien sur la Flam. Sa copie est gardée sur ce PC par prudence : si vous la retirez, éjectez ensuite la Flam avec « Éjecter la Flam » avant de la débrancher.",
        noDevice: "la Flam n'est plus détectée",
      },
    },
    /** Envois prévus à la fin d'une installation (« Installer et envoyer », #609). */
    planned: {
      title: "Envoi prévu après l'installation",
      listLabel: "Histoires à envoyer après leur installation",
      state: {
        installing: "Installation…",
        ready: "Installée, envoi en attente",
        lost: "Ne sera pas envoyée",
        /** Installée, mais la Flam n'a pas été surveillée sans interruption depuis le clic. */
        confirm: "Installée, envoi à confirmer",
      },
      /** Installation en cours, scrutation suspendue depuis le clic. */
      unwatchedHint:
        "La Flam n'a pas pu être surveillée sans interruption (fenêtre réduite ou masquée, lecture en cours) : l'envoi sera à confirmer à la fin de l'installation.",
      confirmHint:
        "La Flam n'a pas pu être surveillée sans interruption depuis la demande (fenêtre réduite ou masquée, lecture en cours) : vérifiez que la Flam branchée est la bonne, puis envoyez l'histoire.",
      lostHint:
        "La Flam a été débranchée ou a changé depuis la demande. L'histoire sera installée dans Mes histoires ; envoyez-la ensuite avec « Envoyer sur la Flam ».",
      /** Retire l'envoi prévu ; l'installation continue. */
      drop: (title: string) => `Ne pas envoyer « ${title} » après l'installation`,
      /** Ligne en échec : Flam débranchée, remplacée ou rebranchée pendant l'installation. */
      deviceChanged:
        "L'histoire est installée dans Mes histoires, mais elle n'a pas été envoyée : la Flam a été débranchée ou a changé pendant l'installation. Vérifiez la Flam branchée, puis utilisez « Envoyer sur la Flam ».",
      /** Ligne en échec : envoi refusé par les règles de « Envoyer sur la Flam » (`reason`). */
      notSent: (reason: string) =>
        `L'histoire est installée dans Mes histoires, mais elle n'a pas été envoyée : ${reason}`,
      announce: {
        planned: (title: string) =>
          `« ${title} » sera envoyée sur la Flam à la fin de son installation`,
        lost: (count: number) =>
          count > 1
            ? "La Flam a été débranchée ou a changé : les histoires en cours d'installation ne seront pas envoyées"
            : "La Flam a été débranchée ou a changé : l'histoire en cours d'installation ne sera pas envoyée",
      },
    },
    finished: {
      title: "Envoi terminé",
      restart:
        "Redémarrez la Flam pour qu'elle reconstruise sa bibliothèque : les nouvelles histoires apparaîtront ensuite.",
    },
    errors: {
      noDevice: "La Flam n'est plus détectée : rien n'a été envoyé.",
      readback:
        "Histoire copiée, mais son titre relu sur la Flam ne se déchiffre pas avec les clés choisies : elle risque d'être illisible. Retirez-la et choisissez une autre histoire officielle.",
      /** Erreur `lunii_needs_v2` : histoire STUdio (format Lunii) et Flam en firmware 1.x. */
      luniiNeedsV2:
        "Les histoires STUdio demandent une Flam en firmware 2.x : cette Flam ne peut pas les lire. Rien n'a été écrit sur la Flam.",
    },
    carrier: {
      title: "Flam en firmware 1.x : mode carrier",
      warningTitle: "Avertissement",
      understood: "J'ai compris",
      continue: "Continuer",
      accepting: "Enregistrement…",
      acceptedAt: (date: string) => `Avertissement accepté le ${date}.`,
      loading: "Lecture de la Flam…",
      loadFailed: (message: string) => `Impossible de lire la Flam : ${message}`,
      sourceLegend: "Clés à utiliser",
      stored: "Clés conservées sur ce PC pour cette Flam",
      fromStory: (title: string) => `Histoire officielle « ${title} »`,
      hiddenHint: "Histoire masquée",
      withStoredHint: "Avec la clé conservée sur ce PC",
      fromEntered: "Saisir la clé (bt)",
      fromEnteredHint:
        "La clé bt d'une histoire officielle de cette Flam, en hexadécimal. flam-launcher retrouve l'histoire qu'elle déchiffre et vérifie tout avant la copie.",
      enteredLabel: "Clé bt",
      enteredHint:
        "64 caractères hexadécimaux (0 à 9, a à f) ; espaces et majuscules acceptés.",
      showEntered: "Afficher la clé",
      checkEntered: "Vérifier la clé",
      checkingEntered: "Recherche de l'histoire…",
      recognized: (title: string) =>
        `Clé reconnue : histoire « ${title} ». Vérifiez que le titre est exact : un titre altéré signale une faute de frappe.`,
      titleConfirmed: "Le titre est exact",
      enteredNotHex: "Seuls les chiffres 0 à 9 et les lettres a à f sont acceptés.",
      enteredLength: (count: number, expected: number) =>
        `${count} caractères hexadécimaux sur ${expected} attendus.`,
      fromFile: "Choisir un fichier…",
      fromFileHint:
        "Fichier bt d'une copie de dossier d'histoire officielle (avec key à côté), ou archive zip qui les contient.",
      pickTitle: "Fichier bt ou archive zip",
      pick: "Parcourir…",
      picked: (path: string) => `Fichier choisi : ${path}`,
      noCandidate:
        "Aucune histoire de cette Flam ne contient de fichier bt : saisissez la clé ou choisissez un fichier.",
      remember: "Conserver ces clés sur ce PC",
      rememberHint:
        "Chiffrées pour votre compte Windows et liées à cette Flam. « Oublier les clés » les supprime.",
      confirm: "Vérifier et envoyer",
      forget: "Oublier les clés",
      forgetting: "Suppression…",
      forgotten: "Clés de cette Flam oubliées sur ce PC.",
      nothingToForget: "Aucune clé conservée pour cette Flam.",
      forgetFailed: (message: string) => `Impossible d'oublier les clés : ${message}`,
    },
    recovery: {
      done: "Un transfert interrompu a été repris : la Flam est de nouveau dans un état cohérent.",
      failed: (message: string) =>
        `La reprise d'un éventuel transfert interrompu a échoué : ${message}`,
    },
    fromLibrary: {
      title: "Envoyer depuis « Mes histoires »",
      listLabel: "Histoires de « Mes histoires » à envoyer",
      empty: "Aucune histoire n'est installée dans « Mes histoires ».",
      loadFailed: "Impossible de lire « Mes histoires ».",
    },
    remove: {
      button: "Retirer",
      title: (title: string) => `Retirer « ${title} » de la Flam ?`,
      description:
        "L'histoire sera supprimée de la Flam. Elle reste dans « Mes histoires » si elle y figure.",
      confirm: "Retirer de la Flam",
      done: (title: string) => `« ${title} » retirée de la Flam.`,
      cleanupPending: " L'espace sera libéré à la prochaine opération.",
      failed: (message: string) => `Impossible de retirer l'histoire : ${message}`,
      busy: "Retrait indisponible pendant un envoi.",
      working: "Retrait…",
      workingNamed: (title: string) => `Retrait de « ${title} » en cours…`,
      otherWorking: "Un retrait est déjà en cours.",
      ejecting: "Retrait indisponible pendant une éjection.",
      firmware:
        "Retrait indisponible : écriture sur cette Flam bloquée (voir l'état du firmware).",
    },
    eject: {
      button: "Éjecter la Flam",
      working: "Éjection en cours…",
      removing: "Éjection indisponible pendant un retrait.",
      busy: "Éjection indisponible pendant un envoi.",
      failed: (message: string) => `Impossible d'éjecter la Flam : ${message}`,
    },
  },
  /** Bandeau expérimental de l'écran « Ma Flam » (#235). */
  deviceBeta: {
    title: "Fonction expérimentale et non officielle",
    risks:
      "L'envoi d'histoires sur la Flam n'est ni prévu ni garanti par son fabricant. Lisez les risques avant de continuer :",
    firmware2: "Firmware 2.x : en test, sauvegardez votre Flam avant.",
    open: "Ouvrir la page des risques",
    openFailed:
      "Ouverture impossible : utilisez « Copier le lien des risques » et collez-le dans votre navigateur.",
    copy: "Copier le lien des risques",
    copied: "Lien copié : collez-le dans votre navigateur.",
    copyFailed: "Copie impossible : sélectionnez l'adresse ci-dessus pour la copier.",
  },
  /** Diagnostic copiable de l'écran « Ma Flam » (#318). */
  deviceDiagnostic: {
    title: "Retour de test",
    intro:
      "Le diagnostic décrit le launcher, Windows et l'état de la Flam, sans clé, numéro de série ni titre d'histoire. Collez-le dans le modèle d'issue « Retour testeur Flam ».",
    copy: "Copier le diagnostic",
    working: "Préparation du diagnostic…",
    copied: "Diagnostic copié",
    failed: (message: string) => `Diagnostic impossible : ${message}`,
    copyFailed:
      "Copie impossible : sélectionnez le texte ci-dessous (Ctrl + A dans la zone) puis copiez-le (Ctrl + C).",
    fieldLabel: "Diagnostic à copier",
    report: "Signaler un problème",
    reportHint:
      "Le modèle d'issue s'ouvre dans votre navigateur (compte GitHub nécessaire) :",
    copyLink: "Copier le lien",
    linkCopied: "Lien copié : collez-le dans votre navigateur.",
    linkCopyFailed: "Copie impossible : sélectionnez l'adresse ci-dessus pour la copier.",
    openFailed:
      "Ouverture impossible : copiez le lien et collez-le dans votre navigateur.",
  },
  toast: {
    region: "Notifications",
  },
  playerScreen: {
    label: "Lecture",
    zone: (title: string) => `Lecteur : ${title}`,
    hint: "Cliquez dans le lecteur ou utilisez « Clavier au lecteur ». Flèches, Entrée, Échap. F6 rend le clavier au launcher.",
    focus: "Clavier au lecteur",
    stop: "Arrêter",
    stopping: "Arrêt en cours…",
    stopFailed: "flam-player n'a pas pu être arrêté.",
    stopTimeout: "flam-player ne répond pas. Réessayez « Arrêter ».",
  },
  theme: {
    label: "Thème",
    system: "Système",
    light: "Clair",
    dark: "Sombre",
  },
  /** État du firmware et garde des écritures sur la Flam (#317). */
  deviceFirmware: {
    unreadable: "illisible",
    family: {
      v1: "1.x",
      v2: "2.x",
      unsupported: "non pris en charge",
      unreadable: "illisible",
    },
    state: {
      v1: "pris en charge (mode carrier)",
      v2: "pris en charge",
      contradiction: "pris en charge, clés à vérifier",
      blocked: "écriture bloquée",
    },
    nonStandard: "fichier .mdf non standard",
    nonStandardTitle: "Fichier .mdf non standard",
    nonStandardBody: (size: number, header: string) =>
      `Le fichier .mdf de cette Flam n'a pas la forme connue (${size} octets, en-tête ${header} ; attendu 126 octets, en-tête 01 00). L'envoi reste possible, mais le risque d'histoire illisible est plus grand : gardez une copie de votre Flam.`,
    blockedTitle: "Écriture sur cette Flam bloquée",
    blocked: {
      device_not_found: "Fichier .mdf absent : la Flam n'est plus détectée.",
      invalid_root: "Ce lecteur n'est pas une Flam détectée.",
      mdf_unreadable: (detail: string) => `Fichier .mdf illisible (${detail}).`,
      truncated: (size: number) => `Fichier .mdf tronqué (${size} octets).`,
      version_unreadable: "Version du firmware illisible dans le fichier .mdf.",
      unsupported_firmware: (version: string) =>
        `Firmware ${version} non pris en charge (seuls les 1.x et 2.x le sont).`,
      keys_unreadable: (detail: string) =>
        `Clés illisibles dans le fichier .mdf (${detail}).`,
      keys_trivial: (version: string) =>
        `Firmware ${version} : clé ou IV vide dans le fichier .mdf.`,
    },
    blockedHint:
      "Rien n'est écrit tant que ce blocage dure. Notez la version et l'état affichés ici dans votre retour de test.",
    contradictionTitle: "Clés de la Flam à vérifier",
    contradiction: (version: string) =>
      `Firmware ${version} : une histoire installée par un autre outil sur cette Flam ne se déchiffre pas avec les clés du fichier .mdf.`,
    contradictionStories: (ids: string[], total: number) => {
      const more = total - ids.length;
      const tail = more > 0 ? ` (et ${more} ${more > 1 ? "autres" : "autre"})` : "";
      return `Histoires en cause : ${ids.join(", ")}${tail}.`;
    },
    contradictionHint:
      "Simple avertissement : l'envoi, le retrait et l'ordre restent possibles. Ce contrôle peut se tromper (histoire d'un autre outil dont le titre est en Latin-1 ou très court). Si une histoire envoyée ne se lit pas sur la Flam, notez la version et l'état affichés ici dans votre retour de test.",
  },
  /**
   * Histoire faite d'un dossier, suites du lot 5 (docs/creation-dossier.md § 8) : erreurs par
   * code, « Choisir le dossier », textes dédiés du rapport.
   */
  authoringFolder: {
    errors: {
      blocked:
        "Le dossier a changé : un point est maintenant à corriger. Rouvrez-le pour le voir.",
      conflictCopied:
        "Ce dossier semble maintenant copié d'un autre dossier d'histoire : rouvrez-le pour choisir « Remplacer » ou « Nouvelle histoire ».",
      conflictForeign:
        "L'identifiant de ce dossier est maintenant celui d'une histoire d'une autre source : rouvrez-le pour créer une nouvelle histoire.",
      versionOverflow:
        "Cette histoire a atteint sa dernière version possible : créez une nouvelle histoire.",
      retryUnavailable:
        "Cette création ne peut plus être relancée : utilisez « Actualiser » sur l'histoire.",
      folderChanged:
        "Le dossier a changé depuis l'échec : utilisez « Actualiser » sur l'histoire.",
      uuidMismatch:
        "Ce dossier n'est pas celui de cette histoire : choisissez le dossier qui contient son fichier _histoire.json.",
      readOnlyFolder:
        "Le dossier de cette histoire était en lecture seule : déplacé, il ne peut pas être reconnu. Créez une nouvelle histoire depuis ce dossier.",
      notFromFolder: "Cette histoire n'a pas été créée depuis un dossier.",
    },
    missingTitle: "Dossier introuvable",
    missing:
      "Le dossier de cette histoire est introuvable : il a peut-être été déplacé, renommé, ou se trouve sur une clé retirée.",
    choose: "Choisir le dossier",
    chooseTitle: "Nouvel emplacement du dossier de l'histoire",
    relocated: "Dossier de l'histoire retrouvé.",
    problem: {
      duplicate_story_name: (name: string) =>
        `« ${name} » : un autre fichier audio porte le même nom, image et titre lu partagés.`,
      fin_ignored: "Réglage « fin » sans effet dans ce mode : retour au menu.",
    },
  },
  /** Créer une histoire à partir d'un dossier (lot 5, docs/creation-dossier.md). */
  authoring: {
    open: "Créer une histoire depuis un dossier",
    refresh: "Actualiser",
    pickTitle: "Dossier de l'histoire",
    reading: "Lecture du dossier…",
    dialogDescription: "Contenu du dossier et points à vérifier avant la création.",
    count: (n: number, chapters: boolean) => {
      const word = chapters ? "chapitre" : "histoire";
      return n <= 1 ? `${n} ${word}` : `${n} ${word}s`;
    },
    fixFirst: "À corriger dans le dossier avant de créer l'histoire :",
    goodToKnow: "Avertissements :",
    more: (n: number) => (n <= 1 ? "Et 1 autre point." : `Et ${n} autres points.`),
    create: "Créer l'histoire",
    update: "Actualiser l'histoire",
    creating: "Création…",
    cancel: "Annuler",
    close: "Fermer",
    started: "Création lancée : l'histoire arrivera dans Mes histoires.",
    copied: {
      legend: "Ce dossier semble copié d'un autre dossier d'histoire déjà installé.",
      newStory: "Nouvelle histoire",
      replace: (title: string) => `Remplacer « ${title} »`,
    },
    foreign:
      "L'identifiant de ce dossier appartient à une histoire d'une autre source : une nouvelle histoire sera créée.",
    failed: "La création n'a pas pu être lancée. Vérifiez le dossier, puis réessayez.",
    folderRefused:
      "Ce dossier ne peut pas être utilisé (disque entier, dossier système, du launcher ou de la Flam). Choisissez un dossier personnel, par exemple dans Documents.",
    folderMissing: "Dossier introuvable : il a peut-être été déplacé ou renommé.",
    busy: "Cette histoire est déjà en cours de création : attendez la fin ou annulez-la.",
    problem: {
      no_story: "Aucun fichier audio dans ce dossier.",
      too_many_stories: "Trop d'histoires ou de chapitres : 300 au maximum.",
      too_many_options: (name: string) =>
        name
          ? `Trop d'éléments dans « ${name} » : 50 au maximum.`
          : "Trop d'éléments à la racine du dossier : 50 au maximum.",
      too_deep: (name: string) =>
        `« ${name} » est trop profond : 5 niveaux de dossiers au maximum.`,
      too_many_entries: "Ce dossier contient trop de fichiers.",
      audio_too_large: "Audio trop volumineux : 3 Gio au maximum.",
      unreadable_folder: (name: string) => `Dossier « ${name} » illisible.`,
      collection_in_book: (name: string) =>
        `« ${name} » est un recueil placé dans un livre à chapitres : non pris en charge.`,
      orphan_image: (name: string) =>
        `Image « ${name} » sans fichier audio du même nom : ignorée.`,
      empty_menu: (name: string) => `Dossier « ${name} » sans audio : ignoré.`,
      title_too_long: (name: string) =>
        `Titre « ${name} » coupé à 40 caractères sur la Flam.`,
      ignored: (name: string) => `« ${name} » ignoré.`,
      settings:
        "Réglage illisible dans _histoire.json ou _dossier.json : valeur par défaut utilisée.",
      other: "Élément du dossier à vérifier.",
    },
  },
  /** Fenêtre de l'éditeur d'histoires (lot 7, docs/creation-dossier.md § 4). */
  authoringEditor: {
    open: "Créer une histoire depuis un dossier",
    edit: "Modifier",
    editHelp: "Ouvre le dossier de cette histoire dans l'éditeur.",
    pickTitle: "Dossier de l'histoire",
    opening: "Ouverture…",
    alreadyOpen:
      "Ce dossier est déjà ouvert dans un éditeur : sa fenêtre est passée au premier plan.",
    openFailed: "L'éditeur n'a pas pu s'ouvrir. Vérifiez le dossier, puis réessayez.",
    closing:
      "L'éditeur de ce dossier vient d'être fermé et termine une opération : réessayez dans un instant.",
    loading: "Lecture du dossier…",
    loadFailed: "Le dossier n'a pas pu être lu.",
    retry: "Réessayer",
    header: {
      cover: "Couverture",
      noCover: "Couverture automatique",
      modeRecueil: "Recueil",
      modeChapitres: "Livre à chapitres",
      age: (age: number) => `Dès ${age} ans`,
      count: (n: number, chapters: boolean) => {
        const word = chapters ? "chapitre" : "histoire";
        return n <= 1 ? `${n} ${word}` : `${n} ${word}s`;
      },
      reveal: "Ouvrir dans l'Explorateur",
      undo: "Annuler",
      redo: "Rétablir",
      undoHelp: (label: string | null) =>
        label ? `Annuler : ${label} (Ctrl+Z)` : "Rien à annuler",
      redoHelp: (label: string | null) =>
        label ? `Rétablir : ${label} (Ctrl+Y)` : "Rien à rétablir",
      create: "Créer l'histoire",
      update: "Mettre à jour l'histoire",
      creating: "Création…",
      blockedHelp: "Corrigez d'abord les points signalés en bas de la fenêtre.",
      started:
        "Création lancée : l'histoire arrivera dans Mes histoires (fenêtre principale du launcher).",
      copiedTitle: "Dossier copié",
      copiedLegend:
        "Ce dossier semble copié d'un autre dossier d'histoire déjà installé.",
      newStory: "Nouvelle histoire",
      replace: (title: string) => `Remplacer « ${title} »`,
      foreign:
        "L'identifiant de ce dossier appartient à une histoire d'une autre source : une nouvelle histoire sera créée.",
      cancel: "Annuler",
      confirmCreate: "Continuer",
    },
    tree: {
      label: "Contenu de l'histoire",
      help: "Flèches pour parcourir, Alt+Flèche haut ou bas pour réordonner, F2 pour renommer, Suppr pour mettre à la corbeille.",
      addFiles: "Ajouter des fichiers",
      newMenu: "Nouveau menu",
      emptyTrash: "Vider la corbeille de l'histoire",
      size: (size: string) => size,
      items: (n: number) => (n <= 1 ? `${n} élément` : `${n} éléments`),
      titleAudio: "Titre lu",
      titleAudioHelp: "Un fichier audio lit le titre.",
      autoImage: "Image automatique",
      autoImageHelp:
        "Pas d'image dans le dossier : une image est choisie automatiquement.",
      problem: "À vérifier",
      blocking: "À corriger",
    },
    panel: {
      rootHeading: "L'histoire",
      menuHeading: "Menu",
      partHeading: "Partie",
      storyHeading: "Histoire",
      chapterHeading: "Chapitre",
      title: "Titre affiché",
      titleHint: (max: number) => `Sur la Flam, le titre est coupé à ${max} caractères.`,
      titleTooLong: (max: number) =>
        `Plus de ${max} caractères : la fin sera coupée sur la Flam.`,
      renameFile: "Renommer aussi le fichier",
      renameFileHint: "Sinon, seul le titre affiché change ; le fichier garde son nom.",
      saveTitle: "Enregistrer le titre",
      storyTitle: "Titre de l'histoire",
      subtitle: "Sous-titre",
      description: "Description",
      age: "Âge conseillé",
      ageHint: "Laisser vide si aucun.",
      ageInvalid: "Âge entre 0 et 99.",
      saveDetails: "Enregistrer",
      image: "Image",
      coverImage: "Couverture",
      imageAlt: (title: string) => `Image de « ${title} »`,
      chooseImage: "Choisir une image…",
      removeImage: "Retirer l'image",
      automatic: "automatique",
      autoImageMenu: "Image de la première histoire de ce menu.",
      autoImageCover: "Image de la première histoire.",
      autoImageStory:
        "Une image sera préparée à la création (pochette du fichier audio, ou image avec le titre).",
      autoImageNone: "Image unie, préparée à la création.",
      keep: "Garder",
      keepHelp: "Copie cette image dans le dossier pour pouvoir la changer ensuite.",
      titleAudio: "Titre lu",
      titleAudioFile: (name: string) => `Fichier : ${name}`,
      titleAudioVoice: "Voix Windows : le titre sera lu automatiquement à la création.",
      listenVoice: "Écouter la voix Windows",
      stopVoice: "Arrêter",
      voiceUnavailable: "La voix Windows n'est pas disponible sur ce PC.",
      chooseAudio: "Choisir un fichier audio…",
      replaceAudio: "Remplacer le fichier…",
      removeAudio: "Retirer le fichier",
      audio: "Fichier audio",
      audioFile: (name: string, size: string) => `${name} (${size})`,
      organize: "Ranger",
      moveUp: "Monter",
      moveDown: "Descendre",
      moveTo: "Déplacer vers",
      move: "Déplacer",
      delete: "Mettre à la corbeille",
      deleteConfirmTitle: (title: string) => `Mettre « ${title} » à la corbeille ?`,
      deleteConfirm:
        "L'élément est rangé dans la corbeille de l'histoire : « Annuler » le remet en place.",
      cancel: "Annuler",
      settings: "Réglages du dossier",
      mode: "Organisation",
      modeInherit: "Comme le dossier parent",
      modeRecueil: "Recueil : un menu de petites histoires",
      modeChapitres: "Livre à chapitres : les chapitres s'enchaînent",
      fin: "À la fin",
      finInherit: "Comme le dossier parent",
      finMenuRecueil: "Retour au menu",
      finSuivante: "L'histoire suivante démarre",
      finMenuChapitres: "Menu des chapitres",
      finFin: "Fin de l'histoire (retour à la bibliothèque)",
      debut: "Au démarrage",
      debutChapitre: "Le premier chapitre démarre",
      debutMenu: "Le menu des chapitres s'ouvre",
      annoncer: "Annoncer le titre avant chaque chapitre",
      annoncerHint: "Seulement pour les chapitres qui ont un titre lu.",
      addHere: "Ajouter des fichiers ici",
      newMenuHere: "Nouveau menu ici",
      newMenuName: "Nom du nouveau menu",
      newMenuDefault: "Nouveau menu",
      create: "Créer",
      applySettings: "Appliquer les réglages",
      problems: "Points à vérifier",
    },
    trash: {
      confirmTitle: "Vider la corbeille de l'histoire ?",
      confirm:
        "Les éléments retirés pendant cette session et les précédentes partent dans la Corbeille Windows. Ce qui en dépend ne pourra plus être annulé ici.",
      empty: "Vider",
      cancel: "Annuler",
      done: (n: number) =>
        n === 0
          ? "La corbeille de l'histoire était déjà vide."
          : n === 1
            ? "1 élément envoyé dans la Corbeille Windows."
            : `${n} éléments envoyés dans la Corbeille Windows.`,
    },
    report: {
      heading: "Rapport",
      none: "Aucun problème : l'histoire peut être créée.",
      blocking: "À corriger avant de créer l'histoire",
      warnings: "Avertissements",
      goTo: (text: string) => `${text} (aller à l'élément)`,
    },
    pick: {
      files: "Ajouter des fichiers audio ou des images",
      image: "Choisir une image",
      audio: "Choisir le titre lu",
      audioFilter: "Fichiers audio",
      imageFilter: "Images",
      mediaFilter: "Audio et images",
    },
    done: (label: string) => `Fait : ${label}.`,
    undone: (label: string) => `Annulé : ${label}.`,
    redone: (label: string) => `Rétabli : ${label}.`,
    skipped: (n: number) =>
      n === 1
        ? "1 fichier n'a pas été copié (format non accepté)."
        : `${n} fichiers n'ont pas été copiés (format non accepté).`,
    /** Surveillance du dossier (lot 9, § 4.5). */
    watch: {
      changed: "Le dossier a changé dans l'Explorateur : l'affichage est à jour.",
      back: "Le dossier est de nouveau accessible : l'affichage est à jour.",
      missing: "Dossier introuvable : l'éditeur passe en lecture seule.",
      missingTitle: "Dossier introuvable",
      missingHelp:
        "Le dossier a été supprimé, renommé ou déplacé, ou la clé USB a été retirée. Rien ne peut être modifié. Rebranchez la clé, ou choisissez le dossier à son nouvel emplacement.",
      choose: "Choisir le dossier",
      pickTitle: "Nouvel emplacement du dossier de l'histoire",
    },
    error: {
      changed:
        "Le dossier a changé entre-temps (modifié ailleurs) : rien n'a été fait. L'affichage est à jour, réessayez.",
      not_undoable:
        "Impossible de défaire cette opération : un fichier a changé depuis. Rien n'a été fait.",
      nothing_to_undo: "Rien à annuler.",
      nothing_to_redo: "Rien à rétablir.",
      gone: "Cet élément n'existe plus dans le dossier. L'affichage est à jour.",
      link: "Élément refusé : c'est un raccourci, une jonction ou un fichier en ligne non téléchargé.",
      source_refused:
        "Fichier refusé : format non accepté, fichier trop gros, ou fichier déjà dans le dossier de l'histoire.",
      too_large: "Trop de fichiers ou fichiers trop gros : rien n'a été copié.",
      settings:
        "Réglage non modifié : le fichier de réglages a été écrit par une version plus récente du launcher.",
      no_recycle_bin:
        "La corbeille n'a pas été vidée : ce disque n'a pas de Corbeille Windows (clé USB, disque réseau).",
      closed:
        "Le dossier a été déplacé ou n'est plus utilisable : fermez cette fenêtre, puis rouvrez-le.",
      refused: (message: string) => `Refusé : ${message}.`,
      inconsistent:
        "L'opération a été interrompue et n'a pu être défaite qu'en partie : vérifiez le dossier dans l'Explorateur.",
      io: "Erreur de lecture ou d'écriture sur le disque. Vérifiez que le dossier est accessible.",
      folderRefused:
        "Ce dossier ne peut pas être utilisé (disque entier, dossier système, du launcher ou de la Flam). Choisissez un dossier personnel, par exemple dans Documents.",
      folderMissing: "Dossier introuvable : il a peut-être été déplacé ou renommé.",
      other: "L'opération n'a pas abouti. Réessayez.",
    },
  },
};

/** Forme commune à toutes les langues. */
export type Messages = typeof fr;
