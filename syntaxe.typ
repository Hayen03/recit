#let ink = rgb("#03350d")
#let muted = rgb("#5F6B7A")
#let accent = rgb("#80372d")
#let accent-dark = rgb("#713b13")
#let accent-soft = rgb("#ffe8e1")
#let rule = rgb("#022f02")
#let paper = rgb("#FBFCFE")

#set document(
  title: "Syntaxe de Récit",
  author: "Récit",
)
#set page(
  paper: "a4",
  margin: (top: 2.2cm, bottom: 2cm, x: 1.9cm),
  fill: paper,
  header: context {
    if counter(page).get().first() > 1 {
      set text(size: 8pt, fill: muted)
      grid(
        columns: (1fr, auto),
        align: (left, right),
        [*RÉCIT*],
        [Syntaxe de Récit],
      )
      v(-2pt)
      line(length: 100%, stroke: .6pt + rule)
    }
  },
  footer: context {
    if counter(page).get().first() > 1 {
      set text(size: 8pt, fill: muted)
      line(length: 100%, stroke: .6pt + rule)
      v(4pt)
      align(center, [#counter(page).display("1")])
    }
  },
)
#set text(
  font: "Libertinus Serif",
  size: 10pt,
  fill: ink,
  lang: "fr",
)
#set par(justify: true, leading: .7em)
#set list(indent: 1.2em, body-indent: .55em, spacing: .28em)
#set enum(indent: 1.2em, body-indent: .55em, spacing: .28em)
#set outline(indent: 1.15em)
#set table(
  inset: (x: 7pt, y: 5pt),
  stroke: .55pt + rule,
  fill: (_, row) => if calc.odd(row) { white } else { rgb("#F6F8FC") },
)

#show heading.where(level: 1, bookmarked: false): it => {
  set align(center)
  v(1.6cm)
  block(
    width: 100%,
    inset: (x: 22pt, y: 30pt),
    radius: 10pt,
    fill: ink,
    stroke: 1pt + accent,
    [
      #text(size: 30pt, weight: "bold", fill: white)[#it.body]
      #v(8pt)
      #line(length: 42mm, stroke: 2pt + accent)
    ],
  )
  v(1.1cm)
}

#show heading.where(level: 2): it => {
  pagebreak(weak: true)
  v(8pt)
  block(
    width: 100%,
    inset: (left: 10pt, top: 7pt, bottom: 7pt),
    fill: accent-soft,
    stroke: (left: 3pt + accent),
    radius: 5pt,
    text(size: 17pt, weight: "bold", fill: accent-dark, it.body),
  )
  v(5pt)
}

#show heading.where(level: 3): it => {
  v(8pt)
  text(size: 13pt, weight: "bold", fill: ink, it.body)
  v(2pt)
  line(length: 100%, stroke: .7pt + rule)
  v(3pt)
}

#show heading.where(level: 4): it => {
  v(5pt)
  text(size: 10.5pt, weight: "bold", fill: accent-dark, it.body)
  v(1pt)
}

#show outline: it => block(
  width: 100%,
  inset: 18pt,
  radius: 7pt,
  fill: white,
  stroke: .7pt + rule,
  [
    //#text(size: 15pt, weight: "bold", fill: ink)[Table des matières]
    #v(9pt)
    #it
  ],
)

#show raw.where(block: true): it => block(
  width: 100%,
  breakable: true,
  inset: 11pt,
  radius: 6pt,
  fill: rgb("#F3F5FA"),
  stroke: .65pt + rule,
  text(font: "DejaVu Sans Mono", size: 8.2pt, fill: rgb("#26344A"), it),
)
#show raw.where(block: false): it => box(
  inset: (x: 3pt, y: 1pt),
  radius: 2pt,
  fill: accent-soft,
  text(font: "DejaVu Sans Mono", size: .88em, fill: accent-dark, it),
)
#show link: set text(fill: accent-dark)
#show emph: set text(fill: accent-dark)
#show strong: set text(weight: "bold", fill: ink)

= Syntaxe de Récit

#outline()

#pagebreak()
Ce document présente la syntaxe requise pour un fichier de récit `.rct`, les champs de chaque éléments, les structures utilisées, les règles de formules, et le déroulement et fonctionnement du moteur (et plus encore).

== Types de données
La syntaxe de récit utilise les types de données suivants:
/ `str` : une chaîne de caractères.
/ `int` : un entier.
/ `float` : un nombre à virgule flottante.
/ `bool` : un booléen.
/ `null` : une valeur nulle.
/ `list` : une liste de valeurs.
/ `dict` : un dictionnaire de paires clé-valeur.
/ `fstr` : Une chaîne de caractères de formattage; avec des formules intégrées. Lorsqu'évaluée, les formules sont remplacées à l'intérieur de la chaîne par leur évaluation. Les formules sont délimitées par `${...}`.
/ `form` : Une chaîne de caractère représentant une seule formule qui sera évaluée pour produire une valeur. Les formules sont délimitées par `${...}`. Pour être considérée comme formule et non comm `fstr`, la formule et ses balises doivent occuper la chaîne entière.

L'environnement d'évaluation d'une formule dépend de où cette formule apparaît dans le récit. Chaque place où une formule peut être utilisée devrait spécifier l'environnement fourni. Si aucun environnement n'est spécifié, assumez l'environnement de base (`recit`, `page`, `connexion`, `test` et `personnage`).

Certaina champs vont exiger qu'une formule s'évalue à un certain type, arrêtant le récit avec une erreur si ce n'est pas le cas.

== Objet Racine
Un fichier `.rct` est essentiellement un fichier JSON5 avec une structure précise.

```json
{
  version: 2,
  titre: "Titre du récit",
  auteur: "Nom de l'auteur",
  synopsis: "Résumé du récit",
  image: null,
  depart: "premiere_page",
  global: {},
  attributs: [],
  tags: {},
  objets: {},
  etats: {},
  statut: "",
  avant: [],
  apres: [],
  pages: {},
  images: {},
  notes: {},
}
```

=== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`version`*], [`int`], [*Oui*], [--], [Version de la syntaxe. La version actuelle est 2.],
[*`titre`*], [`str`], [*Non*], [`null`], [Titre du récit.],
[*`auteur`*], [`str`], [*Non*], [`null`], [Nom de l'auteur.],
[*`synopsis`*], [`str`], [*Non*], [`""`], [Résumé du récit.],
[*`image`*], [`str` | `list` | `null`], [*Non*], [`null`], [Clés des images de titre du récit.],
[*`depart`*], [`str`], [*Oui*], [--], [Page de départ du récit. Il doit y avoir une page avec ce nom dans le champ `pages`.],
[*`global`*], [`dict`], [*Non*], [`{}`], [Variables globales du récit, accessible dans les formules via la variable d'environnement `recit`.],
[*`attributs`*], [`list`], [*Non*], [`[]`\*], [Liste des attributs du récit si la liste par défaut n'est pas désiré.],
[*`tags`*], [`dict`], [*Non*], [`{}`], [Tags narratifs initiaux du récit.],
[*`objets`*], [`dict`], [*Non*], [`{}`], [Définitions des objets du récit.],
[*`etats`*], [`dict`], [*Non*], [`{}`], [Définitions des états du récit.],
[*`statut`*], [`str | fstr | form`], [*Non*], [`""`], [Page de statut personnalisée pour le récit.],
[*`avant`*], [`list`], [*Non*], [`[]`], [Cellules à exécuter avant chaque page du récit.],
[*`apres`*], [`list`], [*Non*], [`[]`], [Cellules à exécuter après chaque page du récit.],
[*`images`*], [`dict`], [*Non*], [`{}`], [Définitions des images du récit.],
[*`pages`*], [`dict`], [*Oui*], [--], [Définitions des pages du récit.]
)

\* Les attributs par défaut sont `force`, `agilite`, `endurance`, `presence`, `chance`, `connaissance` et `perception`. Les doublons sont éliminés.
Un nom d'attribut ne peut pas être un mot réservé de la syntaxe, ni un nom d'objet ou d'état.

=== `global`

Le champs `global` de la racine contient le nom des _constantes globales_ du récit, où la clé d'une valeur est son nom dans les formules (accessible via `recit.cle`).

Lors de l'initialisation du récit, les constantes globales sont _évaluées_ une seule fois, dans leur ordre d'apparition dans le dictionnaire. Dans leur environement, elles ne peuvent accéder qu'à `recit` et les constantes globales précédentes.

Après initialisation, les constantes globales ne peuvent plus être modifiées.

=== Attributs
Les attributs sont des prorpiétés du personnage du récit qui peuvent être modifié au long de l'histoire. Contrairement aux tags narratifs, les attributs peuvent être utilisés dans des _tests de connexion_.

=== État initial du personnage
Une nouvelle partie construit exactement l'état suivant avant l'ouverture de la page `depart`:

#table(columns: (auto, 1fr), stroke: none,
table.header[*Valeur*][*Initialisation*],
[`nom`], [Nom saisi par le joueur; `null` lorsqu'aucun nom n'est fourni par un appel embarqué au moteur.],
[`attributs`], [Une entrée par nom de `attributs`, dans le même ordre, chacune à l'entier `0`.],
[`pv`, `max_pv`], [`12` et `12`.],
[`moral`, `max_moral`], [`12` et `12`.],
[`objets`, `etats`], [Listes vides.],
[`tags`], [Copie indépendante des tags racine après leur évaluation initiale.]
)

Les maximums stockés valent donc 12 au départ; les maximums *effectifs* peuvent ensuite différer à cause des modificateurs d'états. Les valeurs courantes sont bornées entre zéro et leur maximum effectif chaque fois que le moteur les corrige. Le fichier de récit ne possède aucun champ permettant de choisir d'autres valeurs initiales: il faut employer des cellules `avant` ou celles de la page de départ pour les modifier.

=== `tags`
Le champs `tags` contient la définition des _tags narratifs_ initiaux du récit. Les tags peuvent être de n'importe quel type, mais ils sont tous _évalués_ une fois à l'initialisation du personnage.

Ils peuvent être modifiés par la suite avec des _effets de cellules_.

=== `statut`
Le champs `statut` consiste d'une `str`, d'une `fstr` ou d'une formule retournant une `str`. Cette chaîne eest évaluée à chaque fois que le joueur ouvre le panneau de statut du récit, et servira d'affichage personnalisé pour ce panneau.

=== `notes`
Ce champ est ignoré par le moteur et n'a aucun effet sur le déroulement du récit. Par contre, il est conservé par la forge, contrairement à d'autres champs non-définis, ce qui permet de laisser des notes de dévellopeur persistentes.

== Objets d'inventaire
Définis dans le champs `objets` de la racine, les objets d'inventaire sont des éléments que le personnage peut posséder. La clé de la définition dans le dictionnaire agit comme l'_identifiant unique_ de l'objet dans tout le reste du récit.

Une définition d'objet prend la forme suivante:
```json
{
  nom: "Nom d'affichage de l'objet",
  description: "Description de l'objet.",
  traits: [],
  donnees: {}
}
```

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`nom`*], [`str | form | fstr`], [*Non*], [`""`], [Nom d'affichage de l'objet.],
[*`description`*], [`str | form | fstr`], [*Non*], [`""`], [Description d'inventaire de l'objet.],
[*`traits`*], [`list | form`], [*Non*], [`[]`], [Traits pouvant servir à identifier des objets dans les formules ou avec des filtres.],
[*`donnees`*], [`dict`], [*Non*], [`{}`], [Données supplémentaires de l'objet.],
)

=== Nom d'affichage et description
Le nom d'affichage et la description d'un objet sont une `str` ou une autre valeur pouvant s'évaluer en `str` tel qu'une `fstr`. L'environnement d'évaluation de ces champs est l'environnement de base avec la _version limitée_ de `personnage`, et la version limitée de l'objet avec la variable `objet`.

Ces champs sont évalués à chaque fois que l'instance de l'objet est appelée.

=== `traits`
Les traits sont, au final, une liste de `str`. Lors de la définition d'un objet dans le récit, les traits peuvent être également définis comme une `form` qui s'évalue à une `list` de `str`, ou une `list` d'un mix de `str` et de `form` respectant la contrainte précédente. L'environnement d'évaluation de ces champs est l'environnement de base avec la _version limitée_ de `personnage`, et la version limitée de l'objet avec la variable `objet`.

Les traits sont évalués à chaque fois que l'instance de l'objet est appelée.

=== `donnees`
Le champ `donnees` est un dictionnaire de paires clé-valeur pouvant contenir des données supplémentaires sur l'objet. Les valeurs peuvent être de n'importe quel type. Les `form` et `fstr` sont évaluées à l'instanciation de l'objet en question, dans l'environnement de base avec la _version limitée_ de `personnage`, et la version limitée de l'objet avec la variable `objet`.

== États persistants
Définis dans le champs `etats` de la racine, les états persistant sont des conditions pouvant être appliquées au personnage du récit et qui peuvent modifier ses attributs, ses points de vie, et ses résultats lors de tests de connexion. La clé de la définition dans le dictionnaire agit comme l'_identifiant unique_ de l'état dans tout le reste du récit.

```json
{
  nom: "Nom d'affichage de l'état",
  description: "Description de l'état.",
  traits: [],
  donnees: {},
  effets: [],
  avant: [],
  apres: [],
  expiration: null,
  defaite: null,
}
```

=== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`nom`*], [`str | form | fstr`], [*Non*], [`""`], [Nom d'affichage de l'état.],
[*`description`*], [`str | form | fstr`], [*Non*], [`""`], [Description d'inventaire de l'état.],
[*`traits`*], [`list | form`], [*Non*], [`[]`], [Traits pouvant servir à identifier des états dans les formules ou avec des filtres.],
[*`donnees`*], [`dict`], [*Non*], [`{}`], [Données supplémentaires de l'état.],
[*`effets`*], [`list`], [*Non*], [`[]`], [Liste d'effets d'états appliqués à différentes statistiques du personnage.],
[*`avant`*], [`list`], [*Non*], [`[]`], [Liste de cellules à exécuter avant chaque page du récit lorsque l'état est actif.],
[*`apres`*], [`list`], [*Non*], [`[]`], [Liste de cellules à exécuter après chaque page du récit lorsque l'état est actif.],
[*`expiration`*], [`dict | null`], [*Non*], [`null`], [Filtre d’état qui retire automatiquement cette instance à la fin d’une page.],
[*`defaite`*], [`str | fstr | form | null`], [*Non*], [`null`], [Page vers laquelle rediriger si le retrait de cet état provoque une défaite.],
)

=== Nom d'affichage et description
Le nom d'affichage et la description d'un état sont une `str` ou une autre valeur pouvant s'évaluer en `str` tel qu'une `fstr`. L'environnement d'évaluation de ces champs est l'environnement de base avec la _version limitée_ de `personnage`, et la version limitée de l'état avec la variable `etat`.

Ces champs sont évalués à chaque fois que l'instance de l'état est appelée.

=== `traits`
Les traits sont, au final, une liste de `str`. Lors de la définition d'un état dans le récit, les traits peuvent être également définis comme une `form` qui s'évalue à une `list` de `str`, ou une `list` d'un mix de `str` et de `form` respectant la contrainte précédente. L'environnement d'évaluation de ces champs est l'environnement de base avec la _version limitée_ de `personnage`, et la version limitée de l'état avec la variable `etat`.

Les traits sont évalués à chaque fois que l'instance de l'état est appelée.

=== `donnees`
Le champ `donnees` est un dictionnaire de paires clé-valeur pouvant contenir des données supplémentaires sur l'état. Les valeurs peuvent être de n'importe quel type. Les `form` et `fstr` sont évaluées à l'instanciation de l'état en question, dans l'environnement de base avec la _version limitée_ de `personnage`, et la version limitée de l'état avec la variable `etat`.

=== `expiration`
Le champ `expiration` permet de mettre une condition d'expiration sur un état pour qu'il puisse disparaître automatiquement. Il s'agit d'un filtre d'état. Si l'état satisfait son filtre d'expiration, il est automatiquement retiré à la fin de la page. Si le filtre est `null`, l'état ne peut pas expirer automatiquement.

=== `defaite`
Dans le cas où un état met le moral ou les points de vie du personnage à zéro, le champ `defaite` de l'état sera évalué et l'histoire redirigé vers cette page si elle n'est pas nulle.

=== `avant` et `apres`
Les champs `avant` et `apres` sont des listes de cellules à exécuter respectivement avant et après chaque page du récit lorsque l'état est actif. L'environnement d'évaluation de ces cellules est l'environnement de base avec en plus les données de l'état dans la variable `etat`.

=== `images`
Le champ `images` est un dictionnaire où les clés sont des identifiants d'images et les valeurs sont des définitions d'images. Il permet de centraliser la gestion des images utilisées dans le récit.

Une définition d'image peut être de trois types différent: un chemin vers un fichier image (les chemins relatifs partent de l'emplacement du récit), une URL, ou les données de l'image encodées en base64.

Chaque définition est un `dict` contenant au moins un champ `type` qui indique le type de la définition (`fichier`, `url` ou `base64`). Les autres champs dépendent du type choisi. Elles contiennent tous un champ `alt` qui est une `str` décrivant l'image qui sera affichée si l'image ne peut être chargée. Si aucun `alt` n'est fourni, la clé de l'image sera utilisée à sa place.

==== `fichier`
Une définition d'image de type `fichier` contient un champ `chemin` qui indique le chemin vers le fichier image.

==== `url`
Une définition d'image de type `url` contient un champ `url` qui indique l'URL de l'image.

==== `base64`
Une définition d'image de type `base64` contient un champ `donnees` qui contient les données de l'image encodées en base64, ainsi que le champ `format`, une `str`, qui indique le format de l'image (par exemple `"png"` ou `"jpeg"`). Les formats supportés sont `"png"`, `"jpeg"` et `"webp"`.

== Modificateurs

Tous les modificateurs ont une valeur numérique de type `int`, et une catégorie de type `str`.

Les modificateurs de même catégorie ne se cumulent pas; seul le bonus (modificateur positif) le plus élevé, et la pénalité (modificateur négatif) la plus basse sont appliqués. Les modificateurs de catégories différentes se cumulent. Les modificateurs sans catégorie sont considérés comme ayant une catégorie unique et se cumulent avec tous les autres modificateurs.

== Effets d'état
Les effets d'état sont définis à l'aide d'un `dict`. Ils possèdent tous un champ `type` qui contient une `str` représentant le type d'effet. Les autres champs dépendent du type d'effet.

Les différents effets d'état sont les suivants:
#table(columns: (auto, 1fr), stroke: none,
  table.header[*Type*][*Effet*],
  [*`modificateur_attribut`*], [Applique un modificateur numérique à un attribut du personnage.],
  [*`modificateur_test`*], [Applique un modificateur numérique au résultat d'un test de connexion.],
  [*`modificateur_pv_max`*], [Applique un modificateur numérique aux points de vie maximum du personnage.],
  [*`modificateur_moral_max`*], [Applique un modificateur numérique aux points de moral maximum du personnage.],
)

Tout autre type d'effet d'état est considéré comme invalide et provoquera une erreur lors de l'initialisation du récit.

=== Modificateur d'attribut
```json
{
  type: "modificateur_attribut",
  attribut: "nom_de_l_attribut",
  valeur: 0,
  categorie: null
}
```
==== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`type`*], [`str`], [*Oui*], [--], [Type de l'effet d'état. Doit être `modificateur_attribut`.],
[*`attribut`*], [`str` | `form` | `list`], [*Oui*], [--], [Nom de l'attribut à modifier.],
[*`valeur`*], [`int | form`], [*Oui*], [--], [Valeur du modificateur, ou formule qui s'évalue à un `int`.],
[*`categorie`*], [`str | null`], [*Non*], [`null`], [Catégorie du modificateur.],
)

=== Modificateur de test
```json
{
  type: "modificateur_test",
  filtre: null,
  valeur: 0,
  categorie: null
}
```
==== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`type`*], [`str`], [*Oui*], [--], [Type de l'effet d'état. Doit être `modificateur_test`.],
[*`filtre`*], [`dict | null`], [*Non*], [`null`], [Filtre de tests optionnel pour sélectionner les tests à modifier. Sans filtre, tous les tests sont modifiés.],
[*`valeur`*], [`int | form`], [*Oui*], [--], [Valeur du modificateur, ou formule qui s'évalue à un `int`.],
[*`categorie`*], [`str | null`], [*Non*], [`null`], [Catégorie du modificateur.],
)

=== Modificateur de points de vie maximum
```json
{
  type: "modificateur_pv_max",
  valeur: 0,
  categorie: null
}
```
==== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`type`*], [`str`], [*Oui*], [--], [Type de l'effet d'état. Doit être `modificateur_pv_max`.],
[*`valeur`*], [`int | form`], [*Oui*], [--], [Valeur du modificateur, ou formule qui s'évalue à un `int`.],
[*`categorie`*], [`str | null`], [*Non*], [`null`], [Catégorie du modificateur.],
)

=== Modificateur de moral maximum
```json
{
  type: "modificateur_moral_max",
  valeur: 0,
  categorie: null
}
```
==== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`type`*], [`str`], [*Oui*], [--], [Type de l'effet d'état. Doit être `modificateur_moral_max`.],
[*`valeur`*], [`int | form`], [*Oui*], [--], [Valeur du modificateur, ou formule qui s'évalue à un `int`.],
[*`categorie`*], [`str | null`], [*Non*], [`null`], [Catégorie du modificateur.],
)

== Pages
Les pages sont les blocs principaux d'un récit. Elles sont composées de cellules, qui s'exécute et s'affiche dans l'ordre de leur définition. Certains effets et structures peuvent ajouter plus de cellules à exécuter avant ou après la page. L'ordre est généralement le suivant:

1. Cellules héritées de la connexion (si applicable)
2. Cellules du champs `avant` de la racine
3. Cellules héritées du champs `avant` des états actifs (si applicable)
4. Cellules de la page
5. Cellules héritées du champs `après` des états actifs (si applicable)
6. Cellules du champs `après` de la racine

Une fois toutes les cellules exécutées, le moteur vérifie si des états doivent expirer. Finalement, les connexions sont évaluées et affichées.

```json
{
  titre: "Titre de la page",
  tags: {},
  cellules: [],
  connexions: [],
}
```

=== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`titre`*], [`str | fstr | form`], [*Non*], [`""`], [Titre de la page.],
[*`tags`*], [`dict`], [*Non*], [`{}`], [Tags associés à la page.],
[*`cellules`*], [`list`], [*Non*], [`[]`], [Cellules de la page.],
[*`connexions`*], [`list`], [*Non*], [`[]`], [Connexions de la page.],
)

=== `titre`
Le titre de la page, affiché en haut du panneau de la page. Elle est évaluée à chaque fois que la page est affichée, même si seulement partiellement. Si le titre est une formule, elle doit s'évaluer à une `str`.

=== `tags`
Le champs `tags` contient des tags propre à la page. Les valeurs de ces tags sont évaluées récursivement (les formules à l'intérieur de liste et de dictionnaire sont également évaluées) à chaque fois que la page est ouverte, et ne change plus jusqu'à ce qu'elle soit fermée. Ces tags sont disponibles dans diverses formules dans la variable `page` de l'environnement de base.

Pour l'évaluation des tags, l'environnement disponible est composé de `recit` --- les variables globales du récit, `connexion` --- les tags de la connexion qui a mené à cette page (le dictionnaire vide `{}` pour la première page) --- et `test` --- le résultat du test de la connexion (ou `null` s'il n'y en avait aucun).

=== `cellules`
Le champs `cellules` est une liste de `dict` représentant des cellules, et de formules qui s'évaluent à un tel `dict` ou une liste de. Les formules sont évaluées lors du chargement de la page, la liste résultante est applatie et l'ordre des cellules est conservé. Les cellules sont ensuite exécutées dans l'ordre de la liste.

=== `connexions`
Le champs `connexions` est une liste de `dict` représentant des connexions, et de formules qui s'évaluent à un tel `dict` ou une liste de. Les formules sont évaluées après l'exécution de toutes les cellules, la liste résultante est applatie et l'ordre des connexions est conservé. Les connexions sont ensuite exécutées dans l'ordre de la liste.

Si à la fin de l'évaluation des connexions en formules, aucune connexion n'est disponible, le récit se termine.

== Cellules
Une cellule est l’unité ordonnée de narration et d’exécution. La même forme est utilisée à la racine, sur une page, dans une connexion et dans le cycle de vie d’un état.

Tous les endroits qui acceptent une liste de cellules acceptent aussi une seule cellule sous forme de `dict`, qui sera interprété comme une liste contenant cette unique cellule.

```json
{
  conditions: null,
  effets: [],
  texte: "",
  images: [],
}
```

=== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`conditions`*], [`dict | list | null`], [*Non*], [`null`], [Conditions d'affichage de la cellule.],
[*`effets`*], [`dict | list | null`], [*Non*], [`null`], [Effets de la cellule.],
[*`texte`*], [`str | fstr | form | null`], [*Non*], [`null`], [Texte de la cellule.],
[*`images`*], [`list` | `str` | `form` | `null`], [*Non*], [`null`], [Clés des images de la cellule.],
)

Les champ `effets`, `images` et `texte` peuvent être nuls, mais un des trois doit être présent. Une cellule sans texte ne sera pas affichée, mais ses effets seront exécutés.

Les effets sont appliqués avant que le texte ne soit évalué et affiché. Les effets sont appliqués dans l'ordre de leur définition.

=== `conditions`
Une condition, comme pour les connexions, ou une liste de conditions, qui sera interprétée comme une condition de type `et`.
Les effets de la cellule ne seront appliqués et le texte affiché que si la condition est satisfaite. Si le champ `conditions` est nul, la cellule sera toujours appliquée et affichée.

=== `effets`
Un effet de page, ou une liste de. Les effets sont appliqués dans l'ordre de leur définition. Si le champ `effets` est nul, aucun effet ne sera appliqué.

=== `texte`
Le texte de la cellule, qui sera affiché dans le panneau de la page. Le texte est évalué après que les effets de la cellule ont été appliqués. Si le champ `texte` est nul, aucun texte ne sera affiché.

=== `images`
Les images de la cellule, qui seront affichées dans le panneau de la page. Chaque image est référencée par sa clé dans le dictionnaire `images` du récit. Si le champ `images` est nul, aucune image ne sera affichée.

Les images sont affichées après le texte de la cellule, dans l'ordre où elles sont listées.

Ce champ peut être une `str` correspondant à la clé d'une image dans le dictionnaire `images` du récit, une formule, ou une `list` contenant plusieurs de ces éléments. Les formules doivent s'évaluer à une `str`, ou un `list` de `str`.

== Connexions
Une connexion est un lien vers une autre page du récit. Elle peut être conditionnelle, et peut avoir des effets qui s'appliquent lorsque le joueur choisit cette connexion. Il est aussi possible de rajouter un test à la connexion, qui change la destination selon si le test est réussi ou échoué avec un jet de dé.

```json
{
  texte: "",
  destination: "",
  conditions: null,
  test: null,
  tags: {},
  cellules: [],
}
```

=== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`texte`*], [`str | fstr | form`], [*Oui*], [--], [Texte d'affichage de la connexion.],
[*`destination`*], [`str | fstr | form`], [*Oui*], [--], [Destination de la connexion.],
[*`conditions`*], [`dict | list | null`], [*Non*], [`null`], [Conditions d'affichage de la connexion.],
[*`test`*], [`dict | null`], [*Non*], [`null`], [Test de la connexion.],
[*`tags`*], [`dict | null`], [*Non*], [`{}`], [Tags de la connexion.],
[*`cellules`*], [`list | null`], [*Non*], [`null`], [Cellules de la connexion.],
)

=== `texte`
Le texte de la connexion, qui sera affiché dans le panneau de la page. Le texte est évalué au moment de l'affichage. Une formule dans ce champs doit obligatoirement s'évaluer à une `str`.

=== `destination`
La page de destination. Doit s'évaluer à une `str` correspondant à une page existante dans le récit. Ce champ est évalué uniquement lorsqu'elle est suivi par le joueur. Une destination vide ou inconnue provoque une erreur et termine le récit. 

La destination est évaluée après l'exécution du test, mais avant l'évaluation des tags. 

Son environnement d'évaluation est l'environnement de base, mais la variable `test` représente le résultat du test de _cette connexion_ (ou `null` s'il n'y en a pas).

=== `conditions`
Une condition, comme pour les cellules, ou une liste de conditions, qui sera interprétée comme une condition de type `et`. La connexion ne sera affichée que si la condition est satisfaite. Si le champ `conditions` est nul, la connexion sera toujours affichée.

=== `test`
Un test de connexion qui rajoute un peu de hasard à l'histoire. L'attribut du texte sera affiché entre crochet `[...]` après le texte de la connexion. Le test est exécuté uniquement lorsque la connexion est suivie par le joueur.

Le test est exécuté avant l'évaluation des tags et de la destination.

=== `tags`
Le champs `tags` contient des tags propres à la connexion. Les valeurs de ces tags sont évaluées récursivement (les formules à l'intérieur de liste et de dictionnaire sont également évaluées) à chaque fois que la connexion est suivie par le joueur. Ces tags sont disponibles dans diverses formules dans la variable `connexion` de l'environnement de base.

Les tags sont évalués après l'exécution du test et l'évaluation de la destination.

=== `cellules`
Le champs `cellules` est une liste de `dict` représentant des cellules, et de formules qui s'évaluent à un tel `dict` ou une liste de. Les cellules sont évaluées lorsque la connexion est suivie par le joueur et son exécutées lors de la prochaine page, avant les cellules globales et de page. Les cellules sont exécutées dans l'ordre de la liste.

=== Tests de connexion
Les tests rajoutent un peu de hasard au récit. Lorsqu'une connection possède un test et est visitée par le joueur, le moteur génère un nombre aléatoire entre 1 et 20 et y rajoute la valeur d'un attribut du personnage, ainsi que tout autres effets ou modificateurs donnés par les états du personnage. Si le résultat est supérieur ou égal à la valeur de difficulté du test, le test est réussi. Sinon, il est échoué. Il est possible de définir une page ou sera redirigé le joueur en cas d'échec du test, ou de laisser la page de destination inchangée. Le résultat du test sera stocké dans la variable `test` de l'environnement de base, et pourra être utilisé dans les formules de la destination.

```json
{
  attribut: "nom_de_l_attribut",
  difficulte: 0,
  traits: [],
  echec: null,
}
```

=== Champs
#table(columns: (auto, auto, auto, auto, 1fr), stroke: none, 
table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
[*`attribut`*], [`str | form | fstr | null`], [*Non*], [`null`], [Attribut du personnage qui est testé.],
[*`difficulte`*], [`int | form`], [*Non*], [10], [Difficulté du test.],
[*`traits`*], [`list`], [*Non*], [`[]`], [Liste de str donnant un peu plus d'information sur le test.],
[*`echec`*], [`str | fstr | form | null`], [*Non*], [`null`], [Page de destination utilisée en cas d’échec du test.],
)

=== `attribut`
L'attribut du personnage qui est testé. Il doit s'agir d'un attribut existant dans le récit. Le nom de l'attribut est évalué au moment du test, et doit s'évaluer à une `str`. Si l'attribut n'existe pas, l'engin termine avec une erreur. Si le champ est `null`, aucun attribut n'est testé, mais des modificateurs peuvent tout de même s'appliquer.

=== `difficulte`
La difficulté du test. Il s'agit d'un entier, ou d'une formule qui s'évalue à un entier. La difficulté est évaluée au moment du test. Si le résultat du test est supérieur ou égal à la difficulté, le test est réussi. Sinon, il est échoué.

=== `traits`
Une liste de `str` donnant un peu plus d'information sur le test. Les traits sont principalement utilisés par les états pour décider si un modificateur de test s'applique ou non.

=== `echec`
La page de destination en cas d'échec du test. Il s'agit d'une `str`, d'une `fstr` ou d'une formule qui s'évalue à une `str`, ou `null`. Le récit est redirigé vers cette page si le test est échoué. Si le champ est `null`, la page de destination de la connexion est utilisée même en cas d'échec du test.

== Conditions
Une condition détermine si une connexion est affichée ou si une cellule est exécutée. Elle est définie par un `dict` contenant un champ `type`. Une liste de conditions est interprétée comme une condition `et` implicite. Les valeurs numériques peuvent être remplacées par des formules qui s'évaluent au moment du test.

Les différents types de conditions sont les suivants:
#table(columns: (auto, auto, 1fr), stroke: none,
  table.header[*Type*][*Champs*][*Condition satisfaite si...*],
  [*`et`*], [`conditions`], [Toutes les sous-conditions sont satisfaites. Une liste vide est vraie.],
  [*`ou`*], [`conditions`], [Au moins une sous-condition est satisfaite. Une liste vide est fausse.],
  [*`non`*], [`condition`], [La sous-condition n'est pas satisfaite.],
  [*`objet_requis`*], [`filtre`], [Au moins une pile d'objets correspond au filtre d'objets.],
  [*`tag_requis`*], [`filtre`], [Au moins un tag du personnage correspond au filtre de tags.],
  [*`etat_requis`*], [`filtre`], [Au moins un état actif correspond au filtre d'états.],
  [*`attribut_egal`*], [`attribut`, `valeur`], [La valeur effective de l'attribut est égale à `valeur`.],
  [*`attribut_superieur`*], [`attribut`, `valeur`], [La valeur effective de l'attribut est strictement supérieure à `valeur`.],
  [*`attribut_inferieur`*], [`attribut`, `valeur`], [La valeur effective de l'attribut est strictement inférieure à `valeur`.],
  [*`pv_egaux`*], [`valeur`], [Les points de vie sont égaux à `valeur`.],
  [*`pv_superieurs`*], [`valeur`], [Les points de vie sont strictement supérieurs à `valeur`.],
  [*`pv_inferieurs`*], [`valeur`], [Les points de vie sont strictement inférieurs à `valeur`.],
  [*`moral_egal`*], [`valeur`], [Le moral est égal à `valeur`.],
  [*`moral_superieur`*], [`valeur`], [Le moral est strictement supérieur à `valeur`.],
  [*`moral_inferieur`*], [`valeur`], [Le moral est strictement inférieur à `valeur`.],
  [*`tag_page`*], [`filtre`], [Au moins un tag de la page courante correspond au filtre de tags.],
  [*`tag_connexion`*], [`filtre`], [Au moins un tag de la connexion courante correspond au filtre de tags.],
  [*`aleatoire`*], [`poids`], [Un tirage aléatoire réussit avec une probabilité de `poids` pour cent, entre 0 et 100.],
  [*`formule`*], [`formule`], [La formule s'évalue à `true`.],
)

=== `et`, `ou` et `non`
Permet de combiner plusieurs conditions logiques pour déterminer si un ensemble de critères est satisfait. Ils possèdent un champ `conditions` (`condition` pour `non`) qui contient une `list` (une seule condition pour `non`) de conditions enfants. La condition `et` est vrai si toutes les conditions enfants sont satisfaites. La condition `ou` est vraie si au moins une des conditions enfants est satisfaite. La condition `non` est vraie si la condition enfant n'est pas satisfaite.

Exemples de combinaisons logiques:
```json
{ type: "et", conditions: [
  { type: "pv_superieurs", valeur: 0 },
  { type: "non", condition: { type: "etat_requis", filtre: "inconscient" } },
] }
```

=== `objet_requis`, `tag_requis`, `etat_requis`, `tag_page` et `tag_connexion`
Ces conditions possèdent un champ `filtre` contenant respectivement un filtre d'objets, de tags ou d'états. Elles sont satisfaites si au moins un élément de la collection concernée correspond au filtre. `tag_requis` examine les tags du personnage, `tag_page` les tags de la page courante et `tag_connexion` ceux de la connexion entrante.

Les filtres peuvent être des formules qui s'évalue en `dict` ayant la structure du filtre approprié.

=== Conditions d'attribut
Les conditions `attribut_egal`, `attribut_superieur` et `attribut_inferieur` possèdent un champ `attribut` contenant le nom de l'attribut à tester et un champ `valeur`. Elles comparent la valeur effective de l'attribut, modificateurs d'états inclus, à `valeur`.

Les champs `valeur`peuvent contenir une formule, évaluée au moment où la condition est testée.

=== Conditions de points de vie et de moral
Les conditions `pv_egaux`, `pv_superieurs`, `pv_inferieurs`, `moral_egal`, `moral_superieur` et `moral_inferieur` possèdent un champ `valeur` auquel la valeur courante correspondante est comparée.

Les champs `valeur`peuvent contenir une formule, évaluée au moment où la condition est testée.

=== `aleatoire`
La condition `aleatoire` possède un champ `poids` représentant une probabilité en pourcentage, entre 0 et 100. Un nouveau tirage est effectué chaque fois que la condition est évaluée. Elle est satisfaite aléatoirement en fonction de ce poids.

Le champs `poids` peut contenir une formule, évaluée au moment où la condition est testée.

=== `formule`
La condition `formule` possède un champ `formule` qui doit s'évaluer à un `bool`. La condition est satisfaite lorsque la formule retourne `true`.

== Effets de page
Un effet de page est un effet actif pouvant être assigné à une cellule et qui s'applique au personnage. Ils sont définis par un `dict` avec un champ `type` qui contient une `str` représentant le type d'effet. Les autres champs dépendent du type d'effet.

Les différents types d'effets de page sont les suivants:
#table(columns: (auto, 1fr), stroke: none,
  table.header[*Type*][*Effet*],
  [*`ajouter_etat`*], [Rajoute un état persistant au personnage.],
  [*`retirer_etat`*], [Retire un état persistant du personnage.],
  [*`modifier_etat`*], [Modifie un état persistant du personnage.],
  [*`definir_tag`*], [Assigne une valeur à un tag.],
  [*`incrementer_tag`*], [Incrémente la valeur d'un tag.],
  [*`retirer_tag`*], [Retire un tag du personnage.],
  [*`ajouter_objet`*], [Ajoute un objet à l'inventaire du personnage.],
  [*`retirer_objet`*], [Retire un objet de l'inventaire du personnage.],
  [*`modifier_objet`*], [Modifie un objet de l'inventaire du personnage.],
  [*`modifier_pv`*], [Modifie les points de vie du personnage.],
  [*`definir_pv`*], [Assigne les points de vie du personnage à une valeur.],
  [*`modifier_pv_max`*], [Modifie les points de vie maximum du personnage.],
  [*`definir_pv_max`*], [Assigne les points de vie maximum du personnage à une valeur.],
  [*`modifier_moral`*], [Modifie le moral du personnage.],
  [*`definir_moral`*], [Assigne le moral du personnage à une valeur.],
  [*`modifier_moral_max`*], [Modifie le moral maximum du personnage.],
  [*`definir_moral_max`*], [Assigne le moral maximum du personnage à une valeur.],
  [*`modifier_attribut`*], [Modifie un attribut du personnage.],
  [*`definir_attribut`*], [Assigne un attribut du personnage à une valeur.],
  [*`modifier_nom`*], [Modifie le nom du personnage.],
  [*`demander`*], [Pause l'histoire pour poser une question au joueur, et stock sa réponse dans un tag.],
  [*`rediriger`*], [Redirige l'histoire vers une autre page.],
)

=== Champs communs
Tous les effets peuvent contenir un champ optionnel `notification` de type `str`, `fstr` ou `null`. Lorsqu'il est omis, certains effets produisent une notification par défaut; une chaîne vide désactive la notification. Les formules de notification sont évaluées après la mutation et disposent donc du personnage mis à jour. Les variables supplémentaires disponibles dépendent de l'effet et sont décrites ci-dessous.

Les effets qui ciblent une collection (`retirer_etat`, `modifier_etat`, `definir_tag`, `incrementer_tag`, `retirer_tag`, `retirer_objet` et `modifier_objet`) acceptent un champ `selection` valant `"tous"` ou `"premier"`. Sa valeur par défaut est `"tous"`. `"premier"` ne traite que le premier élément correspondant au filtre, dans l'ordre de la collection. Si aucun élément ne correspond, l'effet ne fait rien, sauf le cas particulier d'un filtre de tag `id` décrit plus bas.

Les valeurs indiquées comme pouvant être des formules sont résolues au moment où l'effet est exécuté. Comme les effets d'une cellule sont exécutés dans l'ordre, une formule peut observer les mutations effectuées par les effets précédents.

=== `ajouter_etat`
Ajoute une nouvelle instance d'un état persistant au personnage.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`etat`*], [`str | fstr | form`], [*Oui*], [--], [Identifiant de l'état à instancier. La valeur doit se résoudre à une `str` désignant un état déclaré.],
  [*`donnees`*], [`dict | form | null`], [*Non*], [`{}`], [Métadonnées propres à la nouvelle instance. Elles complètent ou remplacent les données par défaut de l'état.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché après l'ajout.],
)

La notification par défaut est "Tu as reçu l'état: …".

Les formules contenues dans `donnees` sont résolues dans l'environnement du personnage lors de l'instanciation. Plusieurs instances du même état peuvent coexister. Après l'ajout, les PV et le moral sont bornés de nouveau selon leurs maximums effectifs. Si l'état rend le personnage vaincu, sa destination de défaite éventuelle est utilisée.

Dans `notification`, l'environnement contient notamment `etat` (l'identifiant), `nom` (le nom évalué) et `donnees` (les métadonnées de l'instance), en plus de l'environnement de base.

```json
{ type: "ajouter_etat", etat: "empoisonne", donnees: { intensite: 2 } }
```

=== `retirer_etat`
Retire les états actifs qui correspondent à un filtre d'états.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`filtre`*], [`dict | str`], [*Oui*], [--], [Filtre d'états à appliquer. Une `str` est le raccourci d'un filtre par identifiant.],
  [*`selection`*], [`str`], [*Non*], [`"tous"`], [États à retirer: `"tous"` ou seulement le `"premier"`.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message évalué séparément pour chaque état retiré.],
)

La notification par défaut est "Tu as perdu l'état: …".

Les maximums effectifs et les valeurs courantes de PV et de moral sont recalculés après chaque retrait. Le retrait peut donc provoquer une défaite si la disparition d'un modificateur abaisse un maximum; la destination de défaite de l'état retiré est alors utilisée si elle existe. La notification a accès aux mêmes variables d'état que `ajouter_etat`.

=== `modifier_etat`
Fusionne des métadonnées dans chaque état actif sélectionné, sans changer son modèle ni supprimer les clés absentes de `donnees`.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`filtre`*], [`dict | str`], [*Oui*], [--], [Filtre d'états.],
  [*`donnees`*], [`dict | form | null`], [*Non*], [`{}`], [Clés à ajouter ou remplacer dans les métadonnées de chaque état sélectionné.],
  [*`selection`*], [`str`], [*Non*], [`"tous"`], [États à modifier: `"tous"` ou le `"premier"`.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché pour chaque modification.],
)

La notification par défaut est "Tu as modifié l'état: …".

Les formules de `donnees` sont évaluées séparément pour chaque état, dans son environnement. La notification reçoit `ancienne`, le dictionnaire complet des métadonnées avant modification, ainsi que les variables de l'état après modification. Les PV et le moral sont ensuite bornés à nouveau; une modification peut ainsi déclencher la destination de défaite de l'état.

=== `definir_tag` et `incrementer_tag`
Ces effets ciblent les tags narratifs du personnage. `definir_tag` remplace une valeur; `incrementer_tag` lui ajoute `valeur` avec l'opérateur d'addition du langage de formules.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`filtre`*], [`dict | str`], [*Oui*], [--], [Filtre de tags. Une `str` est le raccourci du filtre `{ type: "id", tag: ... }`.],
  [*`chemin`*], [`str | int | list | null`], [*Non*], [`null`], [Chemin relatif à l'intérieur de chaque tag sélectionné.],
  [*`creer_chemin`*], [`bool`], [*Non*], [Voir ci-dessous.], [Autorise la création du tag et des membres intermédiaires manquants.],
  [*`valeur`*], [`any | form`], [*Non*], [`null` | `1`], [Nouvelle valeur ou montant à ajouter.],
  [*`selection`*], [`str`], [*Non*], [`"tous"`], [Tags à traiter: `"tous"` ou le `"premier"`.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [Aucune], [Message évalué pour chaque tag effectivement modifié.],
)

La valeur de `valeur` par défaut est `null` pour `definir_tag` et `1` pour `incrementer_tag`.

`chemin` peut être une chaîne séparée par des points (`"inventaire.or.0"`), un entier, ou une liste de chaînes et d'entiers (`["inventaire", "or", 0]`). Une composante numérique indexe une liste; une autre composante désigne une clé de dictionnaire. Le chemin est une `fstr` et peut donc être construit dynamiquement.

Par défaut, `creer_chemin` vaut `true` lorsque `filtre` est directement un filtre `id`, et `false` pour les autres filtres. Un filtre `id` cible son identifiant même si le tag n'existe pas, ce qui permet de le créer. Un filtre composite ne sélectionne que les tags existants et ne crée rien si aucun ne correspond. Avec `creer_chemin: false`, un tag ou membre manquant provoque une erreur lorsqu'il est effectivement ciblé.

La formule de `valeur` dispose de `tag`, de `valeur` (la valeur avant modification) et, si présent, de `chemin`. Pour `incrementer_tag`, une destination créée commence à `0`; l'ancienne valeur et le montant doivent accepter l'addition. Une notification de `definir_tag` reçoit `tag`, `ancienne`, `ancienne_valeur`, `valeur` (le résultat) et éventuellement `chemin`. Pour `incrementer_tag`, elle reçoit en plus `montant`; `valeur` y représente le résultat final.

```json
{ type: "incrementer_tag", filtre: "reputation", valeur: 2 }
{ type: "definir_tag", filtre: "quete", chemin: ["etapes", 0, "terminee"], creer_chemin: true, valeur: true }
```

=== `retirer_tag`
Retire entièrement les tags sélectionnés, ou seulement un membre imbriqué.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`filtre`*], [`dict | str`], [*Oui*], [--], [Filtre de tags.],
  [*`chemin`*], [`str | int | list | null`], [*Non*], [`null`], [Chemin relatif à retirer dans chaque tag sélectionné. Sans chemin, retire le tag entier.],
  [*`selection`*], [`str`], [*Non*], [`"tous"`], [Tags à traiter.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [Aucune], [Message évalué pour chaque valeur effectivement retirée.],
)

La notification reçoit `tag`, le chemin complet de la valeur retirée, et `valeur`, son ancienne valeur. Une cible inexistante est simplement ignorée.

=== `ajouter_objet`
Instancie un objet déclaré et l'ajoute à l'inventaire.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`objet`*], [`str | fstr | form`], [*Oui*], [--], [Identifiant d'un objet déclaré.],
  [*`quantite`*], [`int | form`], [*Non*], [`1`], [Quantité à ajouter.],
  [*`donnees`*], [`dict | form | null`], [*Non*], [`{}`], [Métadonnées de la nouvelle pile, fusionnées aux données par défaut de l'objet.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché après l'ajout.],
)

La notification par défaut est "Tu as reçu: …".

Une quantité inférieure ou égale à zéro n'ajoute rien et ne produit aucune notification. Les piles compatibles sont normalisées après l'ajout. La notification est évaluée dans l'environnement de la pile ajoutée et expose notamment `objet`, `nom`, `quantite` et `donnees`.

Deux piles sont *compatibles* si, et seulement si, leur identifiant d'objet est identique et si leurs dictionnaires `donnees` effectivement stockés sont égaux récursivement, clés, types et valeurs compris. Le nom, la description et les traits calculés ne participent pas à cette comparaison. La normalisation parcourt l'inventaire de gauche à droite: la première pile de chaque classe compatible conserve sa position et reçoit la somme des quantités; les piles suivantes disparaissent. L'ordre relatif des premières piles distinctes est donc stable. La normalisation ne trie pas l'inventaire et ne supprime pas spécialement une pile dont la somme serait nulle ou négative.

=== `retirer_objet`
Soustrait une quantité à chaque pile d'objets sélectionnée.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`filtre`*], [`dict | str`], [*Oui*], [--], [Filtre d'objets. Une `str` filtre par identifiant.],
  [*`quantite`*], [`int | form`], [*Non*], [`1`], [Quantité maximale retirée de chaque pile.],
  [*`selection`*], [`str`], [*Non*], [`"tous"`], [Piles à traiter.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché pour chaque pile traitée.],
)

La notification par défaut est "Tu as perdu: …".

`quantite` est évaluée séparément dans l'environnement de chaque pile. Il n'est jamais retiré davantage que la quantité disponible; une pile arrivée à zéro est supprimée. La notification reçoit les variables de l'objet retiré, `ancienne` (quantité avant l'effet), `valeur` (quantité restante) et `retires` (quantité réellement retirée).

=== `modifier_objet`
Fusionne des métadonnées dans les piles d'objets sélectionnées.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`filtre`*], [`dict | str`], [*Oui*], [--], [Filtre d'objets.],
  [*`donnees`*], [`dict | form | null`], [*Non*], [`{}`], [Clés à ajouter ou remplacer dans les métadonnées.],
  [*`selection`*], [`str`], [*Non*], [`"tous"`], [Piles à modifier.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché pour chaque pile.],
)

La notification par défaut est "Tu as modifié l'objet: …".

Les formules de `donnees` sont évaluées séparément dans l'environnement de chaque pile. Les clés non mentionnées sont conservées. La notification reçoit les variables de la pile après modification et `ancienne`, le dictionnaire complet de ses métadonnées avant modification. Les piles sont ensuite normalisées.

=== Effets sur les PV et le moral
`modifier_pv` et `modifier_moral` ajoutent `valeur` à la ressource courante. `definir_pv` et `definir_moral` remplacent sa valeur. Les variantes `modifier_pv_max`, `modifier_moral_max`, `definir_pv_max` et `definir_moral_max` agissent de la même manière sur le maximum de base correspondant.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`valeur`*], [`int | form`], [*Non*], [`0`], [Montant ajouté pour un effet `modifier_…`, ou nouvelle valeur pour un effet `definir_…`.],
  [*`defaite`*], [`str | fstr | null`], [*Non*], [`null`], [Page vers laquelle rediriger si l'effet laisse le personnage vaincu.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché si aucune redirection de défaite ne prend priorité.],
)

La notification par défaut est "+/- \<valeur> PV/moral/PV max/moral max"

Les ressources courantes sont toujours bornées entre zéro et leur maximum effectif, modificateurs d'états compris. Un maximum de base est lui-même borné à zéro; modifier un maximum borne immédiatement la valeur courante correspondante. Le personnage est vaincu si ses PV ou son moral atteignent zéro. Lorsque `defaite` est définie et que l'effet entraîne une défaite, sa `fstr` est évaluée et impose la prochaine page; elle dispose de `ancienne` et `valeur`, avant et après bornage. Si `defaite` est absente, la mutation demeure appliquée sans redirection propre à cet effet.

Une notification personnalisée reçoit également `ancienne` et `valeur`. Une chaîne vide la masque. Les notifications par défaut indiquent le montant signé pour les effets `modifier_…`, et la nouvelle valeur pour les effets `definir_…`.

```json
{ type: "modifier_pv", valeur: -3, defaite: "mort" }
{ type: "definir_moral_max", valeur: "${personnage.max_moral + 2}" }
```

=== `modifier_attribut` et `definir_attribut`
`modifier_attribut` ajoute `valeur` à la valeur de base d'un attribut; `definir_attribut` remplace cette valeur de base. Les modificateurs temporaires fournis par les états ne sont pas modifiés.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`attribut`*], [`str`], [*Oui*], [--], [Identifiant d'un attribut déclaré dans le récit.],
  [*`valeur`*], [`int | form`], [*Non*], [`0`], [Montant à ajouter ou nouvelle valeur de base.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché après la modification.],
)
La notification par défaut est "+/- <valeur> attribut".

Une notification personnalisée reçoit `attribut`, `ancienne` et `valeur`. La notification par défaut de `modifier_attribut` affiche le montant signé; celle de `definir_attribut` affiche la nouvelle valeur.

=== `modifier_nom`
Remplace le nom du personnage.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`valeur`*], [`str | fstr`], [*Oui*], [--], [Nouveau nom.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message affiché après le changement.],
)

La notification par défaut est "Ton nom est maintenant: …".

La notification reçoit `ancienne`, qui vaut l'ancien nom ou `"Inconnu"` si aucun nom n'était défini, et `valeur`, le nouveau nom.

=== `demander`
Interrompt l'exécution pour demander une valeur au joueur, valide la réponse selon le type demandé, puis la stocke dans un tag. Après la réponse, l'exécution de la cellule reprend.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`tag`*], [`str | fstr`], [*Oui*], [--], [Chemin du tag de destination. Les membres manquants sont créés.],
  [*`question`*], [`str | fstr`], [*Oui*], [--], [Question affichée au joueur.],
  [*`donnees`*], [`str`], [*Non*], [`"str"`], [Type attendu: `"str"`/`"chaine"`, `"int"`/`"entier"`, `"float"`/`"flottant"` ou `"bool"`/`"booleen"`.],
  [*`defaut`*], [`any | form | null`], [*Non*], [`null`], [Réponse proposée par défaut, du type déclaré.],
  [*`choix`*], [`list | form | null`], [*Non*], [`null`], [Liste fermée de choix pour les chaînes, entiers et flottants.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [`null`], [Message consigné après la réponse.],
)

La notification par défaut est "\<question> > \<réponse>".

Chaque entrée de `choix` est un dictionnaire `{ valeur: ..., texte: ... }`: `valeur` est la valeur réellement stockée et doit correspondre au type `donnees`; `texte` est son libellé affiché et accepte une `fstr`. `choix` peut aussi être une formule qui retourne une telle liste. Une liste vide équivaut à l'absence de choix. Les choix sont ignorés pour un booléen.

La destination, la question, la valeur par défaut et les choix sont évalués dans l'environnement courant. La réponse remplace l'ancienne valeur du tag. Une notification personnalisée reçoit `tag`, `ancienne` et `valeur`; une chaîne vide empêche la consignation du résultat.

```json
{
  type: "demander",
  tag: "identite.age",
  question: "Quel âge as-tu?",
  donnees: "int",
  defaut: 18,
}
```

=== `rediriger`
Impose la page qui sera chargée après l'exécution des effets de la page courante.

#table(columns: (auto, auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Défaut*][*Rôle*],
  [*`destination`*], [`str | fstr`], [*Oui*], [--], [Identifiant de la page de destination.],
  [*`notification`*], [`str | fstr | null`], [*Non*], [Aucune], [Message affiché lors de la redirection.],
)

La destination est évaluée au moment de l'effet et doit identifier une page existante. Si plusieurs effets imposent une destination au cours du même traitement, l'ordre d'exécution détermine celle qui est finalement conservée.

== Filtres de tags
Les filtres de tags sont utilisés pour sélectionner des tags dans des effets de page. Ils sont définis par un `dict` avec un champ `type` qui contient une `str` représentant le type de filtre. Les autres champs dépendent du type de filtre.

Deux raccourcis sont également acceptés:

- une `str` est équivalente à un filtre `id` portant sur cette chaîne;
- une `list` de filtres est équivalente à un filtre `et` contenant cette liste.

Les différents types de filtres de tags sont les suivants:
#table(columns: (auto, 1fr), stroke: none,
  table.header[*Type*][*Effet*],
  [*`et`*], [Accepte un tag si tous les filtres enfants sont acceptés.],
  [*`ou`*], [Accepte un tag si au moins un filtre enfant est accepté.],
  [*`non`*], [Accepte un tag si le filtre enfant n'est pas accepté.],
  [*`id`*], [Accepte un tag si son identifiant correspond à la valeur donnée.],
  [*`valeur_egale`*], [Accepte un tag si sa valeur est égale à la valeur donnée.],
  [*`valeur_inferieure`*], [Accepte un tag si sa valeur est inférieure à la valeur donnée.],
  [*`valeur_superieure`*], [Accepte un tag si sa valeur est supérieure à la valeur donnée.],
  [*`formule`*], [Accepte un tag si la formule donnée s'évalue à `true`.]
)

=== `et`, `ou` et `non`
Ces filtres sont des filtres composés. Ils possèdent un champ `filtres` (`filtre` pour `non`) qui contient une `list` (un seul filtre pour `non`) de filtres enfants. Le filtre `et` accepte un tag si tous les filtres enfants sont acceptés. Le filtre `ou` accepte un tag si au moins un filtre enfant est accepté. Le filtre `non` accepte un tag si le filtre enfant n'est pas accepté.

=== `id`
Le filtre `id` accepte un tag si son identifiant correspond à la valeur donnée. Il contient un champ `tag` qui contient un `str`, une `fstr`, ou une formule qui s'évalue à un `str`.

Le `str` du champ `tag` peut aussi décrire un _chemin_ vers un membre d'un tag. Par exemple, si j'ai dans les tags narratifs la structure suivante:
```json
{
  "tags": {
    "tag1": {
      "tag2": {
        "tag3": 5
      },
      "tag4": [1, 2, 3]
    }
  }
}
```
Alors le filtre `id` avec le champ `tag` égal à `"tag1.tag2.tag3"` acceptera le tag `tag3`, et le filtre `id` avec le champ `tag` égal à `"tag1.tag4"` acceptera le tag `tag4`. Le filtre `id` avec le champ `tag` égal à `"tag1.tag5"` n'acceptera aucun tag, car il n'existe pas. Cela marche également pour les indices de liste, par exemple `"tag1.tag4.0"` acceptera le tag `1` dans la liste, mais le tag `"tag1.tag4.3"` n'acceptera aucun tag, car il n'existe pas.

=== `valeur_egale`, `valeur_inferieure` et `valeur_superieure`
Compare la valeur d'un tag à une valeur donnée dans leur champ `valeur`. Le champ `valeur` peut être de n'importe quel type pour un filtre `valeur_egale`, mais doit être un `int` ou `float` ou une formule qui s'évalue à un `int` ou `float` pour les filtres `valeur_inferieure` et `valeur_superieure`. Le filtre accepte le tag si la comparaison est vraie.

Ces filtres ont également les alias `egal`, `inferieure` et `superieure` respectivement.

=== `formule`
Le filtre `formule` accepte un tag si la formule donnée s'évalue à `true`. La formule en tant que tel est donnée dans le champ `formule`et doit s'évaluer à un `bool`. L'environnement d'évaluation de la formule est l'environnement de base avec la variable `tag` représentant le chemin vers le tag en question, et `valeur` représentant la valeur présentement stocké dans le tag.

== Filtres d'objets
Les filtres d'objets sélectionnent des piles de l'inventaire selon leur identifiant, leur quantité, leurs traits, leurs métadonnées ou une formule. Ils sont notamment utilisés par la condition `objet_requis` et par les effets `retirer_objet` et `modifier_objet`. Le filtre est évalué indépendamment pour chaque pile: deux piles du même objet ayant des métadonnées différentes peuvent donc produire des résultats différents.

La forme complète d'un filtre est un `dict` dont le champ `type` est une `str`. Deux raccourcis sont également acceptés:

- une `str` est équivalente à un filtre `id` portant sur cette chaîne;
- une `list` de filtres est équivalente à un filtre `et` contenant cette liste.

Ainsi, `"potion"` équivaut à `{ type: "id", id: "potion" }`, et `[
  "potion",
  { type: "quantite_superieure", quantite: 1 },
]` sélectionne les piles de plus d'une potion.

Les différents types de filtres d'objets sont les suivants:
#table(columns: (auto, 1fr), stroke: none,
  table.header[*Type*][*Pile acceptée si...*],
  [*`et`*], [Tous les filtres enfants l'acceptent.],
  [*`ou`*], [Au moins un filtre enfant l'accepte.],
  [*`non`*], [Le filtre enfant ne l'accepte pas.],
  [*`id`*], [L'identifiant de son modèle d'objet correspond à l'identifiant demandé.],
  [*`donnee_egale`*], [La métadonnée désignée est égale à la valeur demandée.],
  [*`donnee_superieure`*], [La métadonnée désignée est strictement supérieure à la valeur demandée.],
  [*`donnee_inferieure`*], [La métadonnée désignée est strictement inférieure à la valeur demandée.],
  [*`quantite_egale`*], [Sa quantité est égale à la quantité demandée.],
  [*`quantite_superieure`*], [Sa quantité est strictement supérieure à la quantité demandée.],
  [*`quantite_inferieure`*], [Sa quantité est strictement inférieure à la quantité demandée.],
  [*`tous_les_traits`*], [Elle possède tous les traits demandés.],
  [*`un_des_traits`*], [Elle possède au moins un des traits demandés.],
  [*`formule`*], [La formule s'évalue à `true` dans l'environnement de la pile.],
)

=== `et`, `ou` et `non`
Ces filtres permettent de composer récursivement des critères.

#table(columns: (auto, auto, auto, 1fr), stroke: none,
  table.header[*Type*][*Champ*][*Type du champ*][*Sémantique*],
  [*`et`*], [`filtres`], [`list`], [Vrai si tous les filtres de la liste sont vrais. Une liste vide est vraie.],
  [*`ou`*], [`filtres`], [`list`], [Vrai si au moins un filtre de la liste est vrai. Une liste vide est fausse.],
  [*`non`*], [`filtre`], [`dict | str | list`], [Inverse le résultat de son unique filtre enfant.],
)

Les enfants sont évalués dans leur ordre et bénéficient des mêmes raccourcis que le filtre racine. `et` s'arrête dès qu'un enfant est faux; `ou` s'arrête dès qu'un enfant est vrai. Une erreur de formule rencontrée avant cet arrêt reste une erreur et n'est pas transformée en résultat faux.

```json
{
  type: "et",
  filtres: [
    { type: "un_des_traits", traits: ["consommable", "ingredient"] },
    { type: "non", filtre: { type: "donnee_egale", cle: "maudit", valeur: true } },
  ],
}
```

=== `id`
Le filtre `id` contient un champ obligatoire `id` de type `str`, `fstr` ou formule s'évaluant à une `str`. Il accepte une pile lorsque la valeur résolue est exactement égale à l'identifiant du modèle d'objet dont la pile est une instance. La comparaison porte sur l'identifiant interne, pas sur le nom affiché; elle est sensible à la casse.

La forme abrégée `"identifiant"` est généralement préférable lorsque l'identifiant est statique. Dans la forme complète, une `fstr` peut construire l'identifiant au moment de l'évaluation:

```json
{ type: "id", id: "cle_${personnage.tags.couleur}" }
```

=== `donnee_egale`, `donnee_superieure` et `donnee_inferieure`
Ces filtres comparent une métadonnée de la pile à une valeur.

#table(columns: (auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Rôle*],
  [*`cle`*], [`str`], [*Oui*], [Chemin de la métadonnée à lire.],
  [*`valeur`*], [`any | form`], [*Oui*], [Valeur de comparaison, résolue au moment du test.],
)

`donnee_egale` utilise l'égalité exacte et accepte n'importe quel type de valeur comparable. `donnee_superieure` et `donnee_inferieure` effectuent une comparaison numérique stricte; leur champ `valeur` doit être un `int`, un `float` ou une formule qui produit un nombre. Les bornes ne sont jamais incluses.

Le champ `cle` accepte un chemin séparé par des points. Chaque composante textuelle traverse un `dict`; une composante numérique traverse une `list` par son indice commençant à zéro. Par exemple, `stats.qualite` lit une valeur dans un dictionnaire imbriqué et `charges.0` lit le premier membre d'une liste. Le chemin part toujours de `donnees`, sans qu'il faille écrire ce préfixe. Si une clé, un indice ou une structure intermédiaire n'existe pas, la pile n'est pas acceptée. Le filtre ne crée et ne modifie aucune donnée.

La formule de `valeur` est évaluée dans l'environnement de la pile courante. Elle peut donc dépendre de ses autres métadonnées:

```json
{
  type: "donnee_superieure",
  cle: "durabilite.actuelle",
  valeur: "${durabilite.maximum / 2}",
}
```

=== `quantite_egale`, `quantite_superieure` et `quantite_inferieure`
Ces filtres contiennent un champ obligatoire `quantite` de type `int` ou formule s'évaluant à un `int`.

- `quantite_egale` accepte la pile si sa quantité est exactement égale à la valeur résolue;
- `quantite_superieure` l'accepte si sa quantité est strictement supérieure à cette valeur;
- `quantite_inferieure` l'accepte si sa quantité est strictement inférieure à cette valeur.

La comparaison porte sur la quantité totale de la pile, avant l'application de l'effet qui utilise le filtre. La formule est résolue séparément pour chaque pile dans son environnement. Une quantité n'est pas implicitement bornée par le filtre: une valeur nulle ou négative demeure une valeur de comparaison valide.

```json
{ type: "quantite_superieure", quantite: 3 }
```

=== `tous_les_traits` et `un_des_traits`
Ces filtres contiennent un champ `traits` qui accepte une `str`, une liste de `str`, une formule, ou `null`. Chaque trait peut être une valeur littérale ou être produit par une formule. Les traits du modèle d'objet sont eux-mêmes évalués dans l'environnement de la pile avant la comparaison.

`tous_les_traits` accepte une pile si elle possède chacun des traits demandés. `un_des_traits` l'accepte si elle en possède au moins un. Les comparaisons de traits sont exactes et sensibles à la casse. Par conséquent, une liste vide est acceptée par `tous_les_traits` et refusée par `un_des_traits`. Les alias `tous` et `un` sont aussi reconnus, mais les noms complets sont recommandés.

```json
{ type: "tous_les_traits", traits: ["arme", "magique"] }
{ type: "un_des_traits", traits: "consommable" }
```

=== `formule`
Le filtre `formule` contient un champ obligatoire `formule` qui doit s'évaluer à un `bool`. `true` accepte la pile et `false` la refuse; tout autre type ou toute erreur d'évaluation interrompt l'opération avec une erreur.

La formule est évaluée séparément pour chaque pile dans l'environnement de base enrichi de l'environnement de l'objet. Celui-ci expose:

- `id`, l'identifiant du modèle de l'objet;
- `objet.id`, ce même identifiant;
- `objet.donnees`, le dictionnaire complet des métadonnées;
- `objet.traits`, la liste des traits évalués;
- chaque clé de métadonnée directement comme variable lorsqu'elle est un identifiant utilisable dans une formule.

Pour tester la quantité, les filtres `quantite_egale`, `quantite_superieure` et `quantite_inferieure` offrent une forme portable et explicite. Les variables de l'objet peuvent masquer une variable homonyme de l'environnement extérieur; les valeurs propres à la pile sont donc à privilégier dans ce filtre.

```json
{
  type: "formule",
  formule: "${objet.id == 'potion' and objet.donnees.puissance >= personnage.tags.niveau}",
}
```

== Filtres d'états
Les filtres d'états sélectionnent les instances d'états persistants actuellement actives sur le personnage. Ils peuvent examiner l'identifiant de leur modèle, leurs traits évalués, leurs métadonnées propres ou le résultat d'une formule. Ils sont utilisés par la condition `etat_requis`, par les effets `retirer_etat` et `modifier_etat`, ainsi que par le champ `expiration` d'une définition d'état. Dans ce dernier cas, le filtre est appliqué à l'instance de l'état elle-même à la fin de la page; si elle correspond, elle expire.

Le filtrage porte sur les _instances_, et non uniquement sur les définitions d'états. Plusieurs instances issues du même modèle peuvent donc être examinées séparément et produire des résultats différents lorsque leurs métadonnées diffèrent.

La forme complète d'un filtre est un `dict` dont le champ `type` est une `str`. Deux raccourcis sont acceptés à tous les niveaux:

- une `str` équivaut à un filtre `id` portant sur cette chaîne;
- une `list` de filtres équivaut à un filtre `et` contenant cette liste.

Ainsi, `"empoisonne"` équivaut à `{ type: "id", id: "empoisonne" }`.

Les différents types de filtres d'états sont les suivants:
#table(columns: (auto, 1fr), stroke: none,
  table.header[*Type*][*Instance acceptée si...*],
  [*`et`*], [Tous les filtres enfants l'acceptent.],
  [*`ou`*], [Au moins un filtre enfant l'accepte.],
  [*`non`*], [Le filtre enfant ne l'accepte pas.],
  [*`id`*], [L'identifiant de son modèle d'état correspond à l'identifiant demandé.],
  [*`donnee_egale`*], [La métadonnée désignée est égale à la valeur demandée.],
  [*`donnee_superieure`*], [La métadonnée désignée est strictement supérieure à la valeur demandée.],
  [*`donnee_inferieure`*], [La métadonnée désignée est strictement inférieure à la valeur demandée.],
  [*`tous_les_traits`*], [Elle possède tous les traits demandés.],
  [*`un_des_traits`*], [Elle possède au moins un des traits demandés.],
  [*`formule`*], [La formule s'évalue à `true` dans l'environnement de l'instance.],
)

=== `et`, `ou` et `non`
Ces filtres composent récursivement plusieurs critères.

#table(columns: (auto, auto, auto, 1fr), stroke: none,
  table.header[*Type*][*Champ*][*Type du champ*][*Sémantique*],
  [*`et`*], [`filtres`], [`list`], [Vrai si tous les filtres enfants sont vrais. Une liste vide est vraie.],
  [*`ou`*], [`filtres`], [`list`], [Vrai si au moins un filtre enfant est vrai. Une liste vide est fausse.],
  [*`non`*], [`filtre`], [`dict | str | list`], [Inverse le résultat de son unique filtre enfant.],
)

Les filtres enfants sont évalués dans l'ordre. `et` s'arrête au premier résultat faux et `ou` au premier résultat vrai; les enfants suivants ne sont alors pas évalués. Une erreur rencontrée dans un enfant effectivement évalué est propagée, sauf les cas particuliers des comparaisons numériques décrits plus bas.

```json
{
  type: "et",
  filtres: [
    "empoisonne",
    { type: "donnee_superieure", cle: "intensite", valeur: 2 },
    { type: "non", filtre: { type: "un_des_traits", traits: "benefique" } },
  ],
}
```

=== `id`
Le filtre `id` contient un champ obligatoire `id` de type `str`, `fstr` ou formule s'évaluant à une `str`. Il accepte une instance si la valeur résolue est exactement égale à l'identifiant de sa définition d'état. La comparaison ne porte ni sur le nom affiché ni sur les métadonnées, et elle est sensible à la casse.

Une chaîne utilisée directement comme filtre est la forme abrégée de ce type. La forme complète permet notamment de construire l'identifiant dynamiquement:

```json
{ type: "id", id: "protection_${personnage.tags.element}" }
```

La `fstr` ou formule est évaluée séparément pour chaque instance dans son environnement. Elle peut donc dépendre des métadonnées de l'état visé.

=== `donnee_egale`, `donnee_superieure` et `donnee_inferieure`
Ces filtres lisent une valeur dans les métadonnées propres à l'instance, puis la comparent à une valeur attendue.

#table(columns: (auto, auto, auto, 1fr), stroke: none,
  table.header[*Champ*][*Type*][*Obligatoire*][*Rôle*],
  [*`cle`*], [`str`], [*Oui*], [Chemin de la métadonnée à lire.],
  [*`valeur`*], [`any | form`], [*Oui*], [Valeur attendue, résolue au moment du test.],
)

`donnee_egale` utilise l'égalité exacte et permet toute valeur comparable. `donnee_superieure` et `donnee_inferieure` effectuent une comparaison numérique stricte: leur champ `valeur` doit être un `int`, un `float` ou une formule produisant un nombre. Les bornes ne sont pas incluses.

Le champ `cle` est un chemin séparé par des points qui part directement du dictionnaire `donnees`. Une composante textuelle traverse un `dict`; une composante numérique traverse une `list` par un indice commençant à zéro. Par exemple, `progression.niveau` lit une clé imbriquée et `jets.0` lit le premier élément d'une liste. Il ne faut pas préfixer le chemin par `donnees`.

Si une clé, un indice ou une structure intermédiaire n'existe pas, le filtre retourne `false`. Il ne crée ni ne transforme aucune métadonnée. Pour les variantes numériques, une valeur attendue qui ne peut pas être résolue en nombre produit également `false`; pour `donnee_egale`, une erreur d'évaluation de la formule demeure une erreur.

La formule de `valeur` est évaluée séparément pour chaque instance dans son environnement, ce qui permet de comparer une métadonnée à une autre:

```json
{
  type: "donnee_superieure",
  cle: "duree.restante",
  valeur: "${duree.minimum}",
}
```

Les noms historiques `egale`, `superieure` et `inferieure` sont également reconnus comme alias respectifs, mais les noms complets `donnee_…` sont recommandés.

=== `tous_les_traits` et `un_des_traits`
Ces filtres contiennent un champ `traits` acceptant une `str`, une liste de `str`, une formule, ou `null`. Chaque élément peut être un trait littéral ou une formule qui produit un trait. Les traits définis par le modèle d'état sont eux-mêmes évalués dans l'environnement de chaque instance avant la comparaison; deux instances du même état peuvent donc avoir des traits évalués différemment si ceux-ci dépendent de leurs données.

- `tous_les_traits` accepte l'instance si chacun des traits demandés est présent;
- `un_des_traits` l'accepte si au moins un des traits demandés est présent.

Les comparaisons sont exactes et sensibles à la casse. Avec une liste vide ou `null`, `tous_les_traits` retourne `true` et `un_des_traits` retourne `false`. Les alias `tous` et `un` sont reconnus, mais les noms complets sont recommandés.

```json
{ type: "tous_les_traits", traits: ["magique", "temporaire"] }
{ type: "un_des_traits", traits: "affaiblissement" }
```

=== `formule`
Le filtre `formule` contient un champ obligatoire `formule`. Celui-ci doit être une formule qui s'évalue à un `bool`: `true` accepte l'instance et `false` la refuse. Un autre type de résultat ou une erreur d'évaluation interrompt l'opération.

La formule est évaluée séparément pour chaque instance dans l'environnement de base enrichi de l'environnement de l'état. Lors de l'évaluation normale d'un filtre, celui-ci expose:

- `id`, l'identifiant de la définition d'état;
- `etat.id`, ce même identifiant;
- `etat.donnees`, le dictionnaire complet des métadonnées de l'instance;
- `etat.traits`, la liste des traits évalués de cette instance;
- chaque clé de métadonnée directement comme variable lorsqu'elle constitue un nom utilisable par le langage de formules.

Les variables propres à l'état peuvent masquer des variables homonymes de l'environnement extérieur. Les accès par `etat.donnees` sont donc préférables lorsqu'une ambiguïté est possible. Le nom et la description affichés ne constituent pas des critères dédiés; s'ils doivent influencer un filtre, leur information doit être représentée explicitement par un identifiant, un trait ou une métadonnée.

```json
{
  type: "formule",
  formule: "${etat.id == 'empoisonne' and etat.donnees.intensite > personnage.attributs.endurance}",
}
```

=== Exemples d'utilisation
Une condition vérifiant la présence d'au moins un état négatif d'intensité non nulle:

```json
{
  type: "etat_requis",
  filtre: [
    { type: "un_des_traits", traits: "negatif" },
    { type: "donnee_superieure", cle: "intensite", valeur: 0 },
  ],
}
```

Un effet retirant seulement la première instance temporaire arrivée à échéance:

```json
{
  type: "retirer_etat",
  filtre: {
    type: "et",
    filtres: [
      { type: "tous_les_traits", traits: "temporaire" },
      { type: "donnee_inferieure", cle: "duree", valeur: 1 },
    ],
  },
  selection: "premier",
}
```

Enfin, un filtre placé dans le champ `expiration` d'un état est évalué sur cette même instance. Cet exemple fait expirer l'état lorsque sa donnée `tours_restants` atteint zéro ou moins:

```json
expiration: {
  type: "non",
  filtre: { type: "donnee_superieure", cle: "tours_restants", valeur: 0 },
}
```

== Filtres de tests
Les filtres de tests déterminent si un effet d'état `modificateur_test` s'applique à un test de connexion donné. Ils examinent l'attribut testé, les traits déclarés par le test ou une formule. Un filtre ne réussit ni n'échoue le test et ne modifie aucune donnée: il décide uniquement si la `valeur` du modificateur auquel il appartient doit être prise en compte.

Le filtre est évalué pour chaque effet `modificateur_test` de chaque état actif lors du calcul du modificateur total du test. Il est évalué après la résolution du nom de l'attribut testé, mais avant le lancer du d20 et avant que le résultat du test ne soit connu. Un champ `filtre` absent ou égal à `null` dans `modificateur_test` signifie que le modificateur s'applique à tous les tests.

La forme complète d'un filtre est un `dict` dont le champ `type` est une `str`. Contrairement aux filtres d'objets et d'états, une `str` seule n'est pas un raccourci valide, puisqu'un test n'a pas d'identifiant. Une `list` de filtres est toutefois acceptée comme raccourci d'un filtre `et` contenant cette liste.

Les différents types de filtres de tests sont les suivants:
#table(columns: (auto, 1fr), stroke: none,
  table.header[*Type*][*Test accepté si...*],
  [*`et`*], [Tous les filtres enfants l'acceptent.],
  [*`ou`*], [Au moins un filtre enfant l'accepte.],
  [*`non`*], [Le filtre enfant ne l'accepte pas.],
  [*`attribut_parmi`*], [L'attribut testé appartient à la liste demandée.],
  [*`tous_les_traits`*], [Le test possède tous les traits demandés.],
  [*`un_des_traits`*], [Le test possède au moins un des traits demandés.],
  [*`formule`*], [La formule s'évalue à `true` dans l'environnement du test et de l'état.],
)

=== `et`, `ou` et `non`
Ces filtres permettent de composer récursivement plusieurs critères.

#table(columns: (auto, auto, auto, 1fr), stroke: none,
  table.header[*Type*][*Champ*][*Type du champ*][*Sémantique*],
  [*`et`*], [`filtres`], [`list`], [Vrai si tous les filtres enfants sont vrais. Une liste vide est vraie.],
  [*`ou`*], [`filtres`], [`list`], [Vrai si au moins un filtre enfant est vrai. Une liste vide est fausse.],
  [*`non`*], [`filtre`], [`dict | list`], [Inverse le résultat de son unique filtre enfant.],
)

Les enfants sont évalués dans l'ordre. `et` s'arrête dès qu'un enfant retourne `false`, tandis que `ou` s'arrête dès qu'un enfant retourne `true`; les enfants restants ne sont alors pas évalués. Toute erreur produite par un enfant effectivement évalué est propagée et interrompt le calcul du test.

Une liste utilisée directement est un `et` implicite. Les deux écritures suivantes sont donc équivalentes:

```json
[
  { type: "attribut_parmi", attribut: "force" },
  { type: "tous_les_traits", traits: "physique" },
]

{
  type: "et",
  filtres: [
    { type: "attribut_parmi", attribut: "force" },
    { type: "tous_les_traits", traits: "physique" },
  ],
}
```

=== `attribut_parmi`
Le filtre `attribut_parmi` contient un champ obligatoire `attribut` de type `str` ou `list` de `str`. Il accepte le test si son attribut résolu est exactement égal à l'une des chaînes fournies.

```json
{ type: "attribut_parmi", attribut: "agilite" }
{ type: "attribut_parmi", attribut: ["force", "endurance"] }
```

La comparaison porte sur l'identifiant interne de l'attribut, et non sur un libellé affiché. Elle est sensible à la casse. Une liste vide ne correspond à aucun test. Les entrées sont littérales: ce champ n'interprète ni `fstr` ni formule. Pour calculer dynamiquement les attributs acceptés, il faut employer un filtre `formule`.

Le champ `attribut` du test de connexion peut lui-même être une `fstr` ou une formule. Le filtre reçoit toujours sa valeur finale résolue; il ne compare jamais l'expression source.

=== `tous_les_traits` et `un_des_traits`
Ces filtres contiennent un champ `traits` acceptant une `str`, une `list` de `str`, ou `null`.

- `tous_les_traits` accepte le test si chacun des traits demandés figure dans les traits du test;
- `un_des_traits` accepte le test si au moins un des traits demandés y figure.

Les comparaisons sont exactes et sensibles à la casse. Les traits examinés sont ceux du champ `traits` de la définition du test de connexion; les traits des états, objets ou pages ne sont pas ajoutés implicitement. Une liste vide ou `null` est acceptée par `tous_les_traits` et refusée par `un_des_traits`.

Les valeurs de `traits` sont littérales et ne sont pas évaluées comme des `fstr` ou des formules. Les alias `tous` et `un` sont également reconnus pour les types `tous_les_traits` et `un_des_traits`, mais les noms complets sont recommandés.

```json
{ type: "tous_les_traits", traits: ["physique", "combat"] }
{ type: "un_des_traits", traits: ["social", "intimidation"] }
```

Un filtre `tous_les_traits` exige la présence de tous les éléments demandés, mais le test peut posséder des traits supplémentaires. De même, `un_des_traits` ne demande qu'une intersection non vide; il n'exige pas que les deux listes soient identiques.

=== `formule`
Le filtre `formule` contient un champ obligatoire `formule` qui doit s'évaluer à un `bool`. `true` accepte le test et applique le modificateur; `false` le refuse. Tout autre type de résultat ou toute erreur d'évaluation interrompt le calcul du test avec une erreur.

La formule est évaluée dans l'environnement courant enrichi des informations du test et de l'instance d'état qui fournit le `modificateur_test`. Les variables suivantes sont disponibles:

- `attribut`: identifiant résolu de l'attribut testé;
- `traits`: liste des traits déclarés par le test;
- `test.attribut`: même identifiant d'attribut;
- `test.traits`: même liste de traits;
- `personnage`: vue courante du personnage;
- chaque clé des métadonnées de l'instance d'état, directement comme variable.

L'environnement extérieur de la connexion demeure également accessible, sous réserve des règles normales de masquage des variables. Les variables `attribut` et `traits` ajoutées par le filtre représentent toujours le test et prennent priorité sur d'éventuelles variables homonymes. Les métadonnées de l'état peuvent elles aussi masquer des noms de l'environnement parent avant l'ajout de ces deux variables.

Puisque le filtre est évalué avant le lancer, `test` ne contient à ce moment que `attribut` et `traits`. Les champs de résultat disponibles après un test—comme `resultat`, `jet`, `difficulte`, `modificateur`, `total` et `marge`—ne sont pas encore définis et ne peuvent pas servir à décider si le modificateur s'applique. Le filtre ne peut donc pas dépendre du nombre qui sera lancé.

```json
{
  type: "formule",
  formule: "${attribut == 'endurance' and 'poison' in traits and intensite > 0}",
}
```

Dans cet exemple, `intensite` est une métadonnée de l'instance d'état qui porte le modificateur. Pour consulter les statistiques ou les tags narratifs du personnage sans ambiguïté, il est préférable de passer par `personnage`, par exemple `${personnage.pv < personnage.max_pv}` ou `${personnage.tags.mode == 'nuit'}`.

=== Exemple complet avec `modificateur_test`
L'exemple suivant définit un effet d'état qui ajoute `2` aux tests de force ou d'endurance possédant le trait `physique`, uniquement tant que la métadonnée `intensite` de l'instance est positive:

```json
{
  type: "modificateur_test",
  categorie: "benediction",
  valeur: 2,
  filtre: {
    type: "et",
    filtres: [
      { type: "attribut_parmi", attribut: ["force", "endurance"] },
      { type: "tous_les_traits", traits: "physique" },
      { type: "formule", formule: "${intensite > 0}" },
    ],
  },
}
```

Lorsque le filtre est satisfait, la `valeur` du `modificateur_test` est ensuite évaluée dans le même environnement général, avec `test`, les métadonnées de l'état et `personnage`. Les règles de cumul des catégories de modificateurs s'appliquent après la sélection; elles sont indépendantes du filtre. Un filtre vrai rend donc le modificateur admissible, sans garantir que sa valeur sera additionnée intégralement si d'autres modificateurs de la même catégorie participent au test.

== Formules
Les formules utilisent un langage d'expressions contrôlé. Une formule lit son environnement et retourne une valeur; elle ne modifie le personnage qu'indirectement, par un effet de page.

Une formule est délimitée par `${...}`. Une chaîne de caractères contenant uniquement une formule est évaluée comme telle et retourne la valeur de la formule. Une formule insérée dans un texte plus large est évaluée et remplacée par sa valeur dans le texte final.

La grammaire complète du langage de formules est décrite dans la référence *Formulas* (`syntax.pdf`, source `syntax.typ`), disponible dans le #link("https://github.com/Hayen03/formulas")[dépôt du langage de formules].

=== Mise en évidence et couleurs du texte
Dans les textes affichés par le joueur, `$[texte]` met le contenu en évidence en magenta clair. La forme `$[#RRGGBB|texte]` choisit une couleur de premier plan personnalisée: les six chiffres hexadécimaux représentent le rouge, le vert et le bleu, de `00` à `FF`. Les minuscules et majuscules sont acceptées.

```json
"Une $[lueur] apparaît."
"Une $[#FF8800|lueur orange] apparaît."
"$[#44cc88|Une porte verte] et $[#FF4444|une porte rouge]."
```

Le crochet fermant restaure la couleur du message environnant. Avec `--sans-couleur`, les délimiteurs et les préfixes de couleur valides disparaissent, mais le texte reste affiché.

Seul un préfixe composé de `#`, exactement six chiffres hexadécimaux et `|` sélectionne une couleur. Sinon, tout le contenu est traité comme une mise en évidence ordinaire: `$[#FFF|texte]` affiche donc `#FFF|texte` dans la couleur d'emphase. Les autres barres `|` restent du texte. Un marqueur sans crochet fermant est conservé littéralement; les marqueurs ne s'imbriquent pas et le premier `]` termine le segment.

Les substitutions de formule précèdent la mise en couleur. Si `couleur` contient la chaîne `"#FF8800"`, on peut écrire:

```json
"$[${couleur}|Votre torche faiblit.]"
"$[${'#FF4444' if personnage.pv < 5 else '#44CC88'}|PV : ${personnage.pv}]"
```

La couleur calculée doit être une chaîne au format `#RRGGBB`. Dans une formule, le littéral numérique `#FF8800` est un entier et son interpolation produit un nombre décimal; utilisez la chaîne `'#FF8800'` pour p

== Fonctionnement et déroulement du jeu
Cette section décrit l'ordre observable du moteur. Il est important: les effets modifient le personnage, les formules suivantes voient ces mutations, tandis que les tags de page, de connexion et le résultat d'un test sont des instantanés liés à une transition.

=== Lancement du jeu
Si le jeu est lancé avec un chemin vers un fichier de récit, ce fichier est chargé et utilisé comme source du récit. S'il est lancé avec un répertoire, il exécute:
1. `main.rct` s'il existe dans le répertoire.
2. Le premier fichier de récit trouvé dans le répertoire.
3. Lance une erreur si aucun fichier de récit n'est trouvé.

=== Initialisation
Au chargement, le moteur analyse et valide la structure du récit et ses formules. Une référence statique, structure ou formule invalide produit une erreur contextualisée.

Le dictionnaire `global` est évalué une seule fois, dans son ordre d'apparition. Chaque entrée voit `recit` et les entrées précédentes résolues. Les listes et dictionnaires sont parcourus récursivement; un texte produit par une formule n'est pas réévalué.

Lors d'une nouvelle partie, le moteur demande le nom, crée les attributs et ressources, puis évalue récursivement les tags initiaux avec `recit`. Il ouvre ensuite `depart`. Comme aucune connexion n'a été suivie, `connexion` est `{}` et `test` vaut `null`. Chaque entrée fraîche incrémente le compteur de visites, même une revisite; charger une sauvegarde n'ajoute pas une visite.

=== Menu principal
Lors du lancement du programme, le récit est chargé et le menu principal affiché.

Le menu consiste, dans cet ordre, de:
1. Le titre du récit
2. L'auteur du récit
3. Le synopsis du récit
4. Les images du récit, s'il y en a.

Vient ensuite les 3 options principales du menu.
1. *Commencer* Démare une nouvelle partie
2. *Charger* Reprend une partie sauvegardée
3. *Quitter* Ferme le programme

Si l'utilisateur choisit *Commencer*, l'engin demande à l'utilisateur de rentrer son nom, puis une nouvelle partie est lancée commençant à la page désignée par le champ `depart`, et initialisé avec les tags initiaux.

=== Cycle et environnement d'entrée
```text
choix sur la page A
  → test éventuel
  → destination et tags de connexion
  → entrée dans B
  → phases, rendu et connexions visibles de B
  → nouveau choix
```

Les cellules d'une connexion quittant A sont transportées et exécutées au début de B. À l'entrée, trois instantanés sont construits: `connexion` contient ses tags déjà évalués, `test` son résultat ou `null`, et `page` les tags de B évalués une fois. Pour la page initiale et après une redirection, `connexion` vaut `{}` et `test` vaut `null`.

Les tags de page sont évalués avec `recit`, `connexion` et `test`, volontairement sans le personnage. Ils restent immuables pendant la visite. À cet environnement figé, le moteur superpose une vue fraîche du personnage aux points pertinents; celle-ci reflète les effets déjà appliqués.

=== Ordre exact d'une page fraîche
#table(columns: (auto, auto, 1fr), stroke: none,
  table.header[*Ordre*][*Phase*][*Contenu*],
  [1], [Connexion entrante], [Cellules transportées par la connexion suivie.],
  [2], [Global `avant`], [Cellules du champ racine `avant`.],
  [3], [États `avant`], [Cellules `avant` de chaque instance active, dans leur ordre.],
  [4], [Page], [Cellules de la page courante.],
  [5], [États `apres`], [Cellules `apres` de chaque instance active.],
  [6], [Global `apres`], [Cellules du champ racine `apres`.],
  [7], [Expiration], [Évaluation puis retrait des états expirés.],
  [8], [Rendu], [Titre et messages accumulés.],
  [9], [Connexions], [Génération, conditions et libellés des choix.],
)

Chaque liste conserve son ordre. Les formules de cellules sont évaluées au début de leur phase et peuvent retourner une cellule ou une liste, aplatie à leur position. La liste des états est copiée au début de chaque phase `avant` ou `apres`: un état ajouté pendant une phase n'y participe pas; la phase suivante prend un nouvel instantané.

=== Exécution d'une cellule
Une cellule suit cet ordre:

1. sa condition est évaluée avec le personnage courant et les environnements de page, connexion, test et récit; une cellule d'état reçoit aussi `condition`;
2. si elle est fausse, effets et texte sont tous ignorés;
3. la liste des effets est construite; une formule peut retourner un effet ou une liste;
4. les effets s'exécutent un par un dans l'ordre;
5. après chaque effet, PV et moral sont bornés selon leurs maximums effectifs;
6. notifications et réponses de `demander` sont accumulées;
7. le texte est évalué avec le personnage après les effets, puis accumulé.

Tous les effets dynamiques d'une cellule sont construits avant l'application du premier, mais leurs champs internes sont résolus lors de leur application. La formule qui choisit la liste voit donc le début de la cellule; la `valeur` du second effet voit normalement la mutation du premier.

Le texte n'est pas immédiatement imprimé, mais sa valeur est fixée à cet instant. `demander` affiche d'abord tout le contenu accumulé, pose la question, consigne la réponse, puis reprend.

=== Redirection et défaite
Lorsqu'un effet `rediriger` ou une défaite fournit une destination, les effets suivants de la cellule et toutes les cellules et phases ultérieures sont ignorés. Le texte de la cellule qui a redirigé est néanmoins évalué. L'expiration et le calcul des connexions n'ont pas lieu.

Le contenu déjà accumulé est rendu et le joueur reçoit un unique choix « Continuer ». La destination est ensuite ouverte avec `connexion: {}` et `test: null`. Si le personnage est vaincu, le rendu indique son évanouissement.

Le personnage est vivant seulement si PV et moral sont tous deux supérieurs à zéro. Sans destination de défaite produite, une ressource à zéro n'arrête pas automatiquement la page: le récit doit contrôler la suite par ses conditions et connexions.

=== États et expiration
Les modificateurs d'états sont recalculés lorsque le moteur demande un attribut effectif, un maximum effectif ou un modificateur de test. Ils ne changent pas la valeur de base. Ajouter, modifier ou retirer un état peut donc modifier immédiatement les valeurs dérivées et reborner les ressources.

L'expiration est testée après toutes les phases et seulement sans redirection en attente. Toutes les instances expirées sont d'abord identifiées, puis retirées dans l'ordre. Chaque retrait produit une notification et reborne PV et moral. Si cette baisse entraîne une défaite, le champ `defaite` de l'état expiré peut rediriger.

=== Rendu et connexions
À chaque affichage d'une page (même affichage partiel en cas d'interuption par `demander`), l'affichage précédent du terminal est effacé.

Après les phases, une vue finale du personnage est construite. Le titre est alors évalué; il voit donc tous les effets exécutés. Les textes et notifications avaient été résolus à leur position et ne sont pas recalculés. Le titre est également évalué à tout les affichages partiels causés par l'effet `demander` avec les informations les plus à jour.

Sans redirection, les connexions sont ensuite traitées:

1. les formules de connexion sont évaluées dans l'environnement final et peuvent produire un dictionnaire ou une liste;
2. les résultats sont aplatis en conservant l'ordre;
3. chaque condition est évaluée avec le personnage final;
4. les connexions refusées disparaissent;
5. le texte des restantes est formaté;
6. pour une connexion testée, l'attribut résolu est ajouté entre crochets.

Avec redirection, la seule option affichée est `Continuer`.

Le test n'est pas lancé pendant l'affichage. Si aucune connexion ne reste, le menu propose « Fin ».

=== Sélection et test d'une connexion
Quand le joueur choisit une connexion:

1. son attribut de test éventuel est résolu;
2. sa valeur effective, modificateurs d'attribut compris, est calculée;
3. les `modificateur_test` d'états dont le filtre correspond sont calculés et cumulés par catégorie;
4. le d20 est lancé;
5. la difficulté est résolue;
6. `d20 + attribut effectif + modificateurs` est comparé à la difficulté;
7. l'objet `test` complet est créé;
8. en cas d'échec, `echec` est utilisé s'il existe, sinon la destination normale;
9. la destination est résolue et doit être non vide;
10. les tags de connexion sont évalués une fois avec ce nouveau `test`.

Les cellules de connexion restent différées jusqu'au début de la destination. Elles y voient `test`, `connexion`, `page` et le personnage courant.

=== Règles temporelles des formules
- `recit` est évalué une fois et reste immuable;
- les tags initiaux sont évalués à la création, puis mutables uniquement par effets;
- `page` est évalué une fois par visite;
- `connexion` est évalué une fois après le choix et vaut pour la page suivante;
- `test` est figé après son lancer;
- `personnage` est reconstruit fréquemment et reflète les mutations précédentes;
- cellules, effets et connexions dynamiques sont évalués au début de leur phase respective;
- une `fstr` est formatée quand son champ est consommé;
- aucun résultat n'est réévalué récursivement sans appel explicite à `eval`.

L'ordre influence les valeurs visibles et les tirages pseudoaléatoires. Déplacer une formule, ajouter un court-circuit ou changer l'ordre des cellules peut donc changer la suite aléatoire.

=== Sauvegarde et chargement
Une sauvegarde conserve le personnage, la page et son numéro de visite, le titre et les messages déjà résolus, l'environnement nécessaire et une éventuelle redirection en attente.

Au chargement, le cycle de la page n'est pas rejoué: conditions, effets, questions, textes et tirages déjà accomplis ne se répètent pas. Le contenu rendu est restauré et seul le menu des connexions est régénéré; une redirection en attente restaure « Continuer ». Les définitions d'objets et d'états viennent du récit chargé, leurs données mutables de la sauvegarde.

=== Fin et erreurs
La partie finit lorsqu'aucune connexion n'est disponible et que le joueur confirme « Fin », lorsqu'il n'existe plus de page suivante, ou si le joueur quitte.

Une erreur de formule, type, référence, effet ou structure dynamique interrompt l'opération avec le contexte de page, phase, cellule et effet. Les effets déjà appliqués avant une erreur ne sont pas transactionnels; il faut corriger le récit puis reprendre depuis une sauvegarde ou recommencer.

== Vues de formule et accès aux champs
Les mentions «version limitée» désignent des instantanés en lecture seule, jamais les structures mutables du moteur. L'accès `x.cle` échoue si la propriété n'existe pas; les méthodes `tryget` sont nécessaires pour un accès facultatif.

#table(columns: (auto, 1fr, 1fr), stroke: none,
table.header[*Vue*][*Propriétés par point*][*Variables à plat dans son environnement spécialisé*],
[`personnage`], [`nom`, `pv`, `max_pv`, `moral`, `max_moral`, `attributs`, `objets`, `etats`, `tags`], [Pour les sélections internes: chaque attribut et tag, puis `nom`, `pv`, `max_pv`, `moral`, `max_moral` et `personnage`; l'environnement de base ordinaire n'injecte que `personnage`.],
[`objet` limité à l'exécution], [`id`, `traits`, `donnees`], [`id` et chaque entrée de `donnees`; `traits` reste accessible par `objet.traits`.],
[`objet` résolu], [`id`, `nom`, `description`, `quantite`, `traits`, `donnees` et chaque entrée de `donnees`], [Même environnement limité pendant la résolution de `nom` et `description`.],
[`etat` limité à l'exécution], [`id`, `traits`, `donnees`], [`id` et chaque entrée de `donnees`; `traits` reste accessible par `etat.traits`.],
[`etat` résolu], [`id`, `nom`, `description`, `traits`, `donnees` et chaque entrée de `donnees`], [Même environnement limité pendant la résolution de `nom` et `description`.],
[`tag` sélectionné], [`tag`, `valeur`], [Variables fournies par le filtre de tag seulement.]
)

`personnage.objets` et `personnage.etats` sont des listes de vues résolues. `personnage.attributs` et `personnage.tags` sont des dictionnaires. Les maximums et attributs placés dans cette vue sont déjà effectifs au moment de sa construction. Les collections sont des instantanés: une mutation ultérieure exige de reconstruire l'environnement pour être visible.

La priorité d'un environnement dépend de son contexte. En règle générale, les données propres à l'objet ou l'état sont ajoutées, puis l'environnement de page/personnage est placé au-dessus; un nom déjà présent dans l'environnement extérieur peut donc masquer une métadonnée à plat. L'accès explicite `objet.donnees.cle` ou `etat.donnees.cle` évite cette ambiguïté.

== Format de sauvegarde
La structure de sauvegarde exacte est libre, mais elle doit permettre de reconstruire exactement
- Le personnage et son état complet à un instant donné.
- La page courante et son contenu rendu.
- La prochaine page à afficher (page de redirection) s'il y a lieu.
- Le titre final de la page courrante.
- L'ancien environnement solidifié (nottament l'environement de base au moment de la sauvegarde, moins `recit`).

Un exemple de fichier de sauvegarde pourrait ressembler à ceci:
```json
{
  "personnage": {
    "attributes": {}, "inventory": [],
    "max_hp": 12, "hp": 12,
    "max_morale": 12, "morale": 12,
    "conditions": [], "story_tags": {}, "name": null
  },
  "page": {"page_id": "...", "page_number": 1, "content": []},
  "prochaine_page": null,
  "title": "...",
  "ancien_env": {}
}
```

Une pile de `inventory` a les clés `objet`, `quantite`, `donnees`; un état sauvegardé a `id`, `donnees`. Au chargement, leurs identifiants sont reliés aux définitions du récit actuellement chargé: noms, descriptions, traits, modificateurs et cellules de cycle de vie proviennent donc de cette version du récit. Une définition absente rend la sauvegarde invalide. `page.content` conserve les messages sémantiques déjà rendus, y compris la mise en évidence `$[…]`, mais pas les codes ANSI. `ancien_env` conserve l'environnement solidifié nécessaire à la page restaurée. Les effets et formules déjà exécutés ne sont pas rejoués.

L'état du générateur pseudoaléatoire et la graine *ne sont pas enregistrés*. Charger dans le même processus continue le générateur détenu par ce processus; relancer le programme en chargeant la même sauvegarde repart de la suite créée par `--graine` ou par l'entropie système. Une sauvegarde ne garantit donc pas à elle seule la reproduction des tirages futurs.

== Validation et compatibilité statique
Deux validations distinctes existent:

- `--verifier` analyse le document JSON brut, accumule les problèmes indépendants, puis tente son chargement par le modèle du moteur;
- `--valider` charge directement le modèle et vérifie ses références statiques.

`--verifier` émet `✗ chemin: message` pour une erreur bloquante, `! chemin: message` pour un avertissement et `· chemin: message` pour une information. Il vérifie exhaustivement les catégories suivantes: racine et `version: 2`; types des métadonnées, attributs et dictionnaires; unicité et noms réservés des attributs; présence de `pages` et validité de `depart`; formes des pages, connexions, tests, cellules et effets atomiques; identifiants de cellules; conteneurs différés; types, champs obligatoires et clés propres à chaque effet; familles, types et récursion des conditions et filtres; types de réponses et choix des questions; syntaxe de toute formule complète ou substitution de `fstr`; références statiques vers pages, attributs, objets et états; clés v1 ou anglaises interdites; destinations statiques; pages inatteignables et analyse conservatrice du graphe. Enfin, le même document est passé à `Tale::from_json`; toute incompatibilité restante apparaît au chemin `moteur`.

La validation ne peut pas prouver le type ou la destination d'une formule sans l'exécuter. Une destination dynamique est donc contrôlée syntaxiquement mais traitée conservativement pour l'accessibilité; les erreurs dépendant des valeurs, divisions, indices, arités, données ou branches restent des erreurs d'exécution. Un avertissement ne bloque pas `--verifier`; la présence d'au moins un `✗` produit finalement `Le récit contient des erreurs`.

== Ligne de commande, joueur et Forge
#table(columns: (auto, 1fr), stroke: none,
table.header[*Commande*][*Comportement*],
[`recit RECIT` ou `recit --jouer RECIT`], [Charge, valide les références statiques, puis lance le joueur interactif.],
[`recit --forge RECIT`], [Ouvre la Forge; si le fichier n'existe pas, crée un récit minimal en mémoire et ne l'écrit qu'au premier enregistrement.],
[`recit --verifier RECIT`], [Validation détaillée de Forge et chargement du modèle.],
[`recit --valider RECIT`], [Chargement rapide, résumé des nombres de pages, connexions, objets, états et effets littéraux, puis références statiques.],
[`recit --graphe RECIT SORTIE_DOT`], [Écrit l'analyse conservatrice au format Graphviz DOT.]
)

Les modes sont mutuellement exclusifs. `--graine`/`--seed` prend un entier non signé de 64 bits et ne s'applique qu'au joueur; `--sans-effacement`, `--sans-couleur` et `--debug` également. Les alias anglais `--run`, `--check`, `--dot`, `--validate`, `--seed`, `--no-clear`, `--no-color` sont acceptés. `-h`/`--aide` (`--help`) affiche l'aide et `-V`/`--version` la version.

Dans le joueur, les connexions sont choisies par leur numéro; le menu offre toujours `[S]tatut`, `[I]nventaire`, `[E]tats`, `S[A]uvegarder`, `[C]harger` et `[Q]uitter`, plus `[T]ags` avec `--debug`. «Continuer» est proposé lorsqu'une redirection est déjà en attente et «Fin» lorsqu'aucune connexion ne reste. Les erreurs sont enrichies de leur chaîne de contexte et terminent l'opération courante.

La Forge possède les vues Récit, Pages, Objets, États, Graphe et Problèmes. `Tab`/`Maj+Tab` change de vue; `↑`/`↓` ou `k` déplace la sélection; `Entrée` ou `e` modifie; `a` ou `n` ajoute; `c` copie; `r` renomme; `d` ou `Suppr` supprime; `v` revalide et ouvre Problèmes; `j` édite le JSON complet; `Ctrl+S` enregistre; `F1` ou `?` ouvre l'aide; `Échap` ou `q` quitte hors fenêtre. Dans une fenêtre, `Échap` annule; dans l'éditeur multiligne, `Ctrl+Entrée`, `F2` ou `Ctrl+S` confirme.

La Forge préserve les champs JSON qu'elle ne modifie pas, mais l'enregistrement normalise tout le document en JSON strict indenté par écriture temporaire puis renommage. La mise en forme JSON5, les commentaires, les clés non citées, les chaînes entre apostrophes et les virgules finales ne sont pas préservés. Quitter avec des modifications ouvre exactement les choix «Enregistrer et quitter», «Quitter sans enregistrer» et «Annuler».
