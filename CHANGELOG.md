# Historique

## 1.1

- Ajout de la possibilité de mettre des formules presque partout.
- Unification des connexions littérales et calculées dans `connexions`, avec
  prise en charge des listes mixtes par le moteur, la Forge, la validation et
  le graphe.
- Ajout des tags immuables de connexion, transmis à la page suivante dans
  `connexion`, et des tags de page évalués une seule fois à l'entrée.
- Les conteneurs de cellules et d'effets acceptent désormais une structure
  unique, une formule ou une liste mixte; `demander.choix`, les identifiants
  d'objet ou d'état et les métadonnées d'effet peuvent aussi être calculés.
- La Forge préserve ces formes différées et propose des éditeurs distincts
  pour les structures et les formules, sans remplacer les formes compactes.
- La validation vérifie la syntaxe des formules différées, leurs conteneurs,
  les tags de connexion et de page, ainsi que les alias de filtres acceptés par
  le moteur.
- La Forge migre automatiquement l'ancien champ `connexions_dyn`; le
  validateur signale aussi `connexions_dyn` et l'ancien `contexte` de
  connexion avec une indication de migration.
- Mise à jour de la référence de syntaxe pour l'environnement d'entrée et
  l'ordre exact d'exécution des pages.

## 1.0

- Feature complete
- ajout des captures lexicales minimales aux fonctions anonymes, avec
  `recit`, `personnage` et `page` conservés comme valeurs dynamiques
- bugfixes

## 0.12.0

- Use ref in envs to avoid cloning
- First order function
- `global` field in tale's root

## 0.11.2

- properly send conditions' env to formulas when resolving conditions' name and description

## 0.11.1

- ajout des méthodes de liste `mul`, `unique`, `sort`, `reverse`, `take`, `skip`, `slice`, `chunks`, `windows`, `choice`, `shuffle`, `coalesce`, `tryget`, `weighted_choice`, `indexof`, `enum`, `pop`
- ajout des méthodes d'objet `keys`, `values`, `entries`, `getpath`, `haspath`, `tryget`, `indexof`, `remove`
- ajout des méthodes de chaînes `indexof`, `lower`, `upper`, `trim`, `replace`, `split`, `startswith`, `endswith`, `ccapitalize`, `title`, `slug`, `zfill`
- ajout des méthodes de flottants `round`, `ceil`, `floor`, `abs`, `pow`, `clamp`, `sign`, `lerp`
- ajout des méthodes d'entier `abs`, `pow`, `clamp`, `sign`, `lerp`, `divmod`
- ajout des fonctions globales `type`, `exists`, `fromentries`, `range`, `eval`
- retirer_tag can now follow path, or act on a direct path if the given filter is a single path id filter
- custom status screen
- les connections peuvent prendre un objet de cellule hors d'une liste s'il n'y en a qu'une seule.

## 0.11.0

- ajout du dictionnaire racine `fonctions` pour déclarer des fonctions pures réutilisables dans les formules
- paramètres nommés optionnels avec contrôle d’arité; omission de `parametres` pour les fonctions variadiques
- exposition systématique de la liste des arguments par `args`, avec accès `args.0` ou `args[0]`
- accès à l’environnement de la formule appelante et possibilité d’appeler d’autres fonctions déclarées
- limite de 32 appels imbriqués pour encadrer la récursion directe ou indirecte
- validation au chargement, cache d’AST, éditeur Forge dédié, référence Python et exemple JSON5
- mise à jour complète de la référence de syntax
- ajout des fonctions globales de formule `zip`
- les valeurs des effets de conditions ne sont plus évaluer qu'à l'instanciation, mais à chaque fois que les valeurs actuelles sont calculées

## 0.10.8

- Exposed check traits and attribute to env for check modifier value formulas.
- a path in a tag equal, greater, lesser or id filter will compare the existence and value at that path
- path field in definir_tag and incrementer_tag can now be str
- if the filter is a single id filter, and no path has been defined, definir_tag and incrementer_tag will operate on those values directly, creating the path if requested
- added aliases for some filters
- added dynamic connections
- saving now save the page text to prevent it from changing.
- exposed old metadata of items and conditions to formula's env in modifier_objet and modifier_etat
- added `rediriger` effect

## 0.10.7

- lecture transparente des récits JSON et JSON5 avec `serde_json5`
- prise en charge des commentaires, virgules finales, clés non citées, chaînes à apostrophes et nombres hexadécimaux dans les fichiers de récit
- Forge, validation, graphe et convertisseur v1→v2 adaptés aux fichiers `.json5`
- la Forge normalise toujours ses sauvegardes en JSON strict valide et ne conserve pas les commentaires JSON5
- ajout des opérateurs de formule entiers `&`, `^` et `|`
- ajout des littéraux de formule hexadécimaux (`0x`), binaires (`0b`) et octaux (`0o`)
- référence Python, audits, exemple JSON5 et tests de régression mis à jour

## 0.10.6

- boolean operation in formula (true + false = true)
- save now contains page count
- engine now accepts explicit null check filter
- definir_tag now creates the path by default if the filter is a single tag id filter

## 0.10.5

- ajout du champ optionnel `chemin` à `definir_tag` et `incrementer_tag` pour cibler une entrée imbriquée d’un objet ou d’une liste
- les composantes de chemin acceptent les formules; le chemin complet peut aussi être une formule retournant une liste
- ajout de `creer_chemin` pour autoriser explicitement la création des structures intermédiaires et l’extension des listes
- ajout de `chemin`, `ancienne_valeur` et, pour l’incrément, `montant` dans l’environnement de notification des tags
- ajout de `--debug` et de l’option de menu `[T]ags`
- ajout de `[C]harger` au menu en jeu pour remplacer la partie courante sans quitter le moteur
- Forge, validation, référence Python, documentation et tests mis à jour

## 0.10.4

- les destinations de connexion acceptent maintenant les substitutions de formule (`$tag`, `${...}` et formes mixtes)
- résolution des destinations dynamiques au moment où la connexion est choisie, avec accès au personnage et aux tags de la page courante
- ajout de la fonction de formule `pow(base, exposant)`
- Forge permet de choisir une page statique ou de saisir une destination dynamique
- validation et graphe adaptés aux destinations impossibles à résoudre statiquement
- tests de régression Rust et Python

## 0.9.8

- bugfixes
- formatting changes
- correction du parsing des chaînes-formules dans les dictionnaires de filtre de `personnage.selectionne`

## 0.9.7

- bugfixes

## 0.9.6

- bugfixes
- removed dependency to lazy_static and regex

## 0.9.5

- bugfixes

## 0.9.4

- bugfixes

## 0.9.3

- bugfixes
- changé les environments disponible à certaines evaluation

## 0.9.2

- bugfixes
- changé les environments disponible à certaines evaluation

## 0.9.1

- bugfixes

## 0.9.0 - Changement des notifications

- les notifications sont maintenant des champs des variantes de PageEffect
- toutes les notifications sont calculés lorsque l'effet est appliqué, puis envoyé au moteur

## 0.8.1 — interface française canonique

- remplacement des clés v2 provisoires par `avant`, `apres` et `libelle`;
- renommage de la condition de page en `tag_page`;
- francisation des variables de notification en `personnage`, `objet`, `etat`,
  `valeur` et `ancienne`;
- commandes et aide en français en premier, avec les anciens noms anglais
  conservés uniquement comme alias de compatibilité;
- mise à jour coordonnée du moteur, de la Forge, du convertisseur, des exemples,
  des validations et de la documentation;
- ajout de contrôles qui refusent les graphies anglaises dans la syntaxe v2.

## 0.8.0 — syntaxe de récit version 2

- ajout du champ racine obligatoire `version: 2` et des tags narratifs initiaux;
- normalisation de `avant`, `apres`, des pages et des connexions sous forme de cellules;
- remplacement des traits de page par des tags valués et immuables;
- ajout de la condition `tag_page` fondée sur les filtres de tag;
- ajout des notifications d’effet personnalisées, silencieuses et par cible;
- migration complète de la Forge, de sa validation et des exemples vers la v2;
- ajout du convertisseur autonome `recit-v1-vers-v2`;
- mise à jour de la référence de syntaxe et des commentaires de documentation.

## 0.7.1 — documentation Rust

- ajout de commentaires Rustdoc aux modules, structures, énumérations, traits, fonctions et méthodes du crate;
- ajout d’un audit automatisé de la couverture documentaire;
- documentation des constantes publiques et du type de terminal de la Forge;
- correction de deux virgules finales dans les exemples JSON de la référence de syntaxe.

## 0.6.0

- remaniement de flot d'affichage pour la version Rust
- ajout de l'effet "demander" pour la version Rust

## 0.5.1

- bugfixes (lots of)
- ajout de `len` au moteur Rust

## 0.5.0

- ajout d'effets de fin de page
- ajout de texte de groupe d'effet
- effet `retirer_tag` et `modifier_nom`
- passage des traits de page à plus de filtre et effet
- résolution des données par défaut des objets lors de la création d’une pile;
- notation par point pour les dictionnaires dans le moteur Python;
- ajout des propriétés françaises de personnage;
- deuxième vérification de l’expiration des états après les effets de fin;
- prise en charge des effets de fin globaux et de page dans la forge;
- ajout de len au moteur Python—though this should wait until Rust also supports it.

## 0.4.0

- passage à `EvalValue` à l'interne

## 0.3.0 — projet unifié

- fusion du moteur et de la Forge du récit dans un seul crate;
- ajout du binaire unique `recit`;
- lecture par défaut avec `recit fichier.json`;
- ajout de `--run` et de son alias français `--jouer`;
- ajout de `--forge` pour ouvrir l'éditeur TUI;
- ajout de `--check`, `--validate` / `--valider` et `--dot`;
- validation de la forge contre le même `model::Tale` que le joueur;
- conservation du document JSON brut pour l'édition sans perte;
- intégration de la syntaxe filtrée de `definir_tag` et `incrementer_tag`;
- ordre d'effets connexion → page → global;
- ajout d'un workflow de compilation des exécutables étudiants pour quatre plateformes.

## 0.2.0 — forge

- fenêtres modales Ratatui pour tous les menus et champs;
- éditeur simple et multiligne intégré;
- annulation transactionnelle par `Échap`;
- conservation de l'écran derrière les fenêtres.

## 0.2.0 — moteur

- effets globaux et groupes conditionnels;
- traits de page et conditions de traits;
- environnement `page`;
- écran alternatif et suppression du défilement précédent.

## 0.1.0

- premiers portages Rust séparés du moteur et de la forge.
