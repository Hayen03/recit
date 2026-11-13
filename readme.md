# Récit

**Récit** est un moteur de jeu d'histoire dont vous êtes le héros. À l'aide d'une structure expressive et d'un langage de formule complexe, il est possible de créer et de jouer à toutes sortes d'histoires.

Les histoires sont construites dans des fichiers JSON5 permettant un bon contrôle à l'écrivain et l'éditeur.

Il est possible de sauvegarder une partie, puis de la reprendre par la suite.

## Lancement du programme

Pour lancer le programme, il suffit de donner le chemin vers l'histoire (fichier `.json`, `.json5`, ou `.rct`) en argument de ligne de commande. Si l'histoire comporte plusieurs fichier, le chemin donné peut être un dossier contenant les fichiers nécessaires.

```txt
./recit karaleir
```

## Forge

La forge est une sous-application avec TUI permettant de facilement créer une histoire sans manipuler le fichier JSON5 à la main.

```txt
./recit --forge nouveau_recit.rct
```

## Karaleir

L'histoire de la jungle de Karaleir est disponible comme démmonstration et tests des fonctionnalités du moteur.

L'histoire n'est présentement pas terminée.
