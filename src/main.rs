//! Command-line entry point for playing, validating, editing, and exporting tales.
//!
//! The selected mode is resolved before any interactive terminal setup. Player display
//! flags do not affect Forge, validation, or graph export.
use anyhow::{bail, Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser};
use recit::{
    app::App, document::TaleDocument, effects::{Deferred}, engine::Game, error_report::print_error,
    graph, load_tale, terminal, validation::{Severity, validate_tale}
};
use std::{path::{Path, PathBuf}, process::ExitCode};

#[derive(Debug, Parser)]
#[command(
    name = "recit",
    version,
    about = "Joue ou édite un récit textuel JSON ou JSON5",
    long_about = "Un seul exécutable pour jouer un récit JSON/JSON5 avec le moteur de référence ou l'éditer dans la Forge du récit.\n\nSans option de mode, le fichier positionnel est joué.",
    help_template = "{about-with-newline}\nUtilisation : {usage}\n\nArguments :\n{positionals}\n\nOptions :\n{options}"
)]
/// Defines the command-line options accepted by the `recit` executable.
struct Arguments {
    /// Joue ce récit (mode par défaut avec un argument positionnel)
    #[arg(value_name = "RECIT", conflicts_with_all = ["run", "forge", "check", "dot", "validate_file"])]
    tale_file: Option<PathBuf>,

    /// Joue ce récit avec le moteur de référence
    #[arg(long = "jouer", alias = "run", value_name = "RECIT", conflicts_with_all = ["tale_file", "forge", "check", "dot", "validate_file"])]
    run: Option<PathBuf>,

    /// Ouvre ce récit dans la Forge TUI; le crée s'il n'existe pas
    #[arg(long, value_name = "RECIT", conflicts_with_all = ["tale_file", "run", "check", "dot", "validate_file"])]
    forge: Option<PathBuf>,

    /// Vérifie le JSON ou JSON5 avec la forge et le modèle du moteur sans le lancer
    #[arg(long = "verifier", alias = "check", value_name = "RECIT", conflicts_with_all = ["tale_file", "run", "forge", "dot", "validate_file"])]
    check: Option<PathBuf>,

    /// Exporte le graphe du récit au format Graphviz DOT
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    #[arg(long = "graphe", alias = "dot", num_args = 2, value_names = ["RECIT", "SORTIE_DOT"], conflicts_with_all = ["tale_file", "run", "forge", "check", "validate_file"])]
    dot: Option<Vec<PathBuf>>,

    /// Charge le récit dans le moteur et valide ses références
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    #[arg(long = "valider", alias = "validate", value_name = "RECIT", conflicts_with_all = ["tale_file", "run", "forge", "check", "dot"])]
    validate_file: Option<PathBuf>,

    /// Graine optionnelle pour le générateur de nombres aléatoires
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    #[arg(long = "graine", alias = "seed", value_name = "GRAINE", conflicts_with_all = ["forge", "check", "dot", "validate_file"])]
    seed: Option<u64>,

    /// Ne pas effacer le terminal entre les écrans du jeu
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    #[arg(long = "sans-effacement", alias = "no-clear", conflicts_with_all = ["forge", "check", "dot", "validate_file"])]
    no_clear: bool,

    /// Désactiver les couleurs ANSI dans le jeu
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    #[arg(long = "sans-couleur", alias = "no-color", conflicts_with_all = ["forge", "check", "dot", "validate_file"])]
    no_color: bool,

    /// Active les outils de débogage du menu en jeu
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    #[arg(long = "debug", conflicts_with_all = ["forge", "check", "dot", "validate_file"])]
    debug: bool,
}

/// Loads a tale and starts the interactive player with the selected runtime options.
///
/// Processes events synchronously and assumes the caller has already initialized the
/// appropriate player or Forge terminal session.
fn run_game(path: PathBuf, arguments: &Arguments) -> Result<()> {
    let path = find_tale(&path)?;
    let tale = load_tale(&path)
        .with_context(|| format!("Impossible de lancer le récit '{}'", path.display()))?
        .with_root(tale_directory(&path));
    let reference_errors = tale.validate_references();
    if !reference_errors.is_empty() {
        eprintln!(
            "Le récit contient {} référence(s) invalide(s):",
            reference_errors.len()
        );
        for error in reference_errors {
            eprintln!("- {error}");
        }
        bail!("Le récit ne peut pas être lancé");
    }

    //let old_wdir = std::env::current_dir()?;
    //std::env::set_current_dir(tale_directory(&path))
    //    .with_context(|| format!("Impossible d'accéder au répertoire du récit '{}'", path.display()))?;

    let res = Game::new(
        tale,
        arguments.seed,
        !arguments.no_clear,
        !arguments.no_color,
        arguments.debug,
    )
    .run()
    .with_context(|| format!("Le récit '{}' s'est interrompu", path.display()));

    //std::env::set_current_dir(&old_wdir)?;
    res
}

/// Returns the directory against which tale-relative resources are resolved.
///
/// A bare filename has an empty parent path, which must mean the current directory rather
/// than being passed directly to `set_current_dir` (where it produces OS error 2).
fn tale_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Finds the path to the tale file:
/// 1. If the path is a file, return it as is.
/// 2. If the path is a directory, returns the first .rct file found within it (prioritizing main.rct).
/// 3. If none was found, returns an error.
fn find_tale(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if path.is_dir() {
        let mut main_rct = None;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            //println!("{}", path.display());
            if path.extension().and_then(|ext| ext.to_str()) == Some("rct") {
                if path.file_name().and_then(|name| name.to_str()) == Some("main.rct") {
                    return Ok(path);
                }
                if main_rct.is_none() {
                    main_rct = Some(path);
                }
            }
        }
        if let Some(main_rct) = main_rct {
            return Ok(main_rct);
        }
    }
    bail!("Aucun fichier .rct trouvé dans '{}'", path.display());
}

/// Loads or creates a tale document and starts the Forge TUI.
///
/// Processes events synchronously and assumes the caller has already initialized the
/// appropriate player or Forge terminal session.
fn run_forge(path: PathBuf) -> Result<()> {
    let document = TaleDocument::load_or_new(path.clone())
        .with_context(|| format!("Impossible d'ouvrir {}", path.display()))?;
    let mut terminal = terminal::TerminalSession::enter()?;
    let result = App::new(document).run(&mut terminal.terminal);
    terminal
        .restore()
        .context("Impossible de restaurer le terminal après la Forge")?;
    result.with_context(|| format!("La Forge du récit '{}' s'est interrompue", path.display()))
}

/// Validates a tale and exits with an error when blocking problems are found.
///
/// Processes events synchronously and assumes the caller has already initialized the
/// appropriate player or Forge terminal session.
fn run_check(path: PathBuf) -> Result<()> {
    if !path.exists() {
        bail!("Le récit '{}' n'existe pas", path.display());
    }
    let document = TaleDocument::load_or_new(path.clone())
        .with_context(|| format!("Impossible d'ouvrir {}", path.display()))?;
    let problems = validate_tale(&document.root);
    if problems.is_empty() {
        println!("Aucun problème détecté dans {}", path.display());
        return Ok(());
    }
    for problem in &problems {
        println!(
            "{} {}: {}",
            problem.severity.symbol(),
            problem.path,
            problem.message
        );
    }
    if problems
        .iter()
        .any(|problem| problem.severity == Severity::Error)
    {
        bail!("Le récit contient des erreurs");
    }
    Ok(())
}

/// Exports the tale page graph to a Graphviz DOT file.
///
/// Processes events synchronously and assumes the caller has already initialized the
/// appropriate player or Forge terminal session.
fn run_dot(paths: Vec<PathBuf>) -> Result<()> {
    let [input, output]: [PathBuf; 2] = paths.try_into().map_err(|_| {
        anyhow::anyhow!("--graphe exige exactement un récit et un fichier de sortie")
    })?;
    if !input.exists() {
        bail!("Le récit '{}' n'existe pas", input.display());
    }
    let document = TaleDocument::load_or_new(input.clone())
        .with_context(|| format!("Impossible d'ouvrir {}", input.display()))?;
    std::fs::write(&output, graph::to_dot(&document.root))
        .with_context(|| format!("Impossible d'écrire {}", output.display()))?;
    println!("Graphe exporté vers {}", output.display());
    Ok(())
}

/// Prints validation problems and returns whether the tale is executable.
///
/// Processes events synchronously and assumes the caller has already initialized the
/// appropriate player or Forge terminal session.
fn run_validation(path: PathBuf) -> Result<()> {
    let tale = load_tale(&path)?;
    let connections: usize = tale.pages.values().map(|page| page.connections.len()).sum();
    let global_effects: usize = tale
        .pre
        .iter()
        .filter(|dc| matches!(dc, Deferred::Literal(_)))
        .chain(tale.post.iter().filter(|dc| matches!(dc, Deferred::Literal(_))))
        .map(|dc| match dc {
            Deferred::Literal(lit) => lit,
            _ => unreachable!(),
        })
        .map(|cell| cell.effects.len())
        .sum();
    let page_effects: usize = tale
        .pages
        .values()
        .flat_map(|page| page.cells.iter()
            .filter(|dc| matches!(dc, Deferred::Literal(_)))
            .map(|dc| match dc {
                Deferred::Literal(lit) => lit,
                _ => unreachable!(),
            }))
        .map(|cell| cell.effects.len())
        .sum();
    let connection_effects: usize = tale
        .pages
        .values()
        .flat_map(|page| page.connections.iter()
            .filter_map(|dc| match dc {
                Deferred::Literal(lit) => Some(lit),
                _ => None,
            })
        )
        .flat_map(|connection| connection.cells.iter()
            .filter_map(|dc| match dc {
                Deferred::Literal(lit) => Some(lit),
                _ => None,
            })
        )
        .map(|cell| cell.effects.len())
        .sum();
    let errors = tale.validate_references();
    println!("Récit: {}", tale.title);
    println!("Pages: {}", tale.pages.len());
    println!("Connexions: {connections}");
    println!("Objets: {}", tale.items.len());
    println!("États: {}", tale.conditions.len());
    println!("Images: {}", tale.images.len());
    println!(
        "Effets: {}",
        global_effects + page_effects + connection_effects
    );
    if errors.is_empty() {
        println!("Validation: OK");
        Ok(())
    } else {
        println!("Validation: {} erreur(s)", errors.len());
        for error in errors {
            println!("- {error}");
        }
        bail!("Le récit contient des références invalides")
    }
}

/// Parses command-line arguments and dispatches to the requested program mode.
fn try_main() -> Result<()> {
    let matches = Arguments::command()
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            clap::Arg::new("aide")
                .short('h')
                .long("aide")
                .alias("help")
                .action(clap::ArgAction::Help)
                .help("Affiche l'aide de la ligne de commande"),
        )
        .arg(
            clap::Arg::new("version_programme")
                .short('V')
                .long("version")
                .action(clap::ArgAction::Version)
                .help("Affiche la version du programme"),
        )
        .get_matches();
    let arguments = Arguments::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());

    if let Some(path) = arguments.forge.clone() {
        return run_forge(path);
    }
    if let Some(path) = arguments.check.clone() {
        return run_check(path);
    }
    if let Some(paths) = arguments.dot.clone() {
        return run_dot(paths);
    }
    if let Some(path) = arguments.validate_file.clone() {
        return run_validation(path);
    }
    if let Some(path) = arguments
        .run
        .clone()
        .or_else(|| arguments.tale_file.clone())
    {
        return run_game(path, &arguments);
    }

    bail!("Aucun récit fourni. Utilisez `recit --aide` pour voir les commandes.")
}

/// Runs the selected command and renders the complete error chain on failure.
fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_error(&error);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tale_directory;
    use std::path::Path;

    #[test]
    fn bare_tale_filename_uses_current_directory() {
        assert_eq!(tale_directory(Path::new("karaleir.rct")), Path::new("."));
    }

    #[test]
    fn tale_with_directory_uses_its_parent() {
        assert_eq!(
            tale_directory(Path::new("stories/karaleir.rct")),
            Path::new("stories")
        );
    }
}
