use std::{io::Read, path::{Path, PathBuf}, sync::{Arc, mpsc::{Receiver, Sender}}, time::Duration};
use anyhow::Context;
use base64::Engine;
use image::DynamicImage;
use serde_json::Value;
use std::collections::HashMap;
use anyhow::Result;

const MAX_IMAGE_SIZE: usize = 10 * 1024 * 1024; // 10 MB
pub(crate) const SUPPORTED_FORMATS: &[&str] = &["png", "jpeg", "webp"];

#[derive(Debug, Clone)]
pub enum ImageDefinition {
    Path(PathBuf),
    Url(Arc<str>),
    Base64{data: Arc<str>, format: Arc<str>},
}

impl ImageDefinition {
    pub fn decode(&self, client: &reqwest::blocking::Client, (sender, receiver): (&Sender<Option<DynamicImage>>, &Receiver<Option<DynamicImage>>), root: &Path) -> Option<DynamicImage> {
        match self {
            Self::Path(path) => {
                let full_path = if path.is_absolute() {
                    path.clone()
                } else {
                    root.join(path)
                };
                image::open(full_path).ok()
            },
            Self::Url(url) => {
                spawn_image_download(client.clone(), url.clone(), sender.clone());
                receiver.recv().ok().flatten() // blocks
            },
            Self::Base64{data, format} => {
                let decoded = base64::engine::general_purpose::STANDARD.decode(&data[..]).ok()?;
                let image = image::load_from_memory_with_format(&decoded, image::ImageFormat::from_extension(format.as_ref())?).ok()?;
                Some(image)
            },
        }
    }
}

pub fn download_image(client: &reqwest::blocking::Client, url: &str) -> anyhow::Result<DynamicImage> {
    let response = client
        .get(url)
        .timeout(Duration::from_millis(1000))
        .send()
        .with_context(|| format!("Impossible de télécharger l'image '{url}'"))?
        .error_for_status()
        .with_context(|| format!("Le serveur a refusé l'image '{url}'"))?;

    if let Some(length) = response.content_length() {
        if length > MAX_IMAGE_SIZE as u64 {
            anyhow::bail!("L'image dépasse la taille maximale autorisée ({length} / {MAX_IMAGE_SIZE})");
        }
    }

    let mut reader = response.take(MAX_IMAGE_SIZE as u64 + 1);
    let mut bytes = Vec::new();

    reader
        .read_to_end(&mut bytes)
        .context("Impossible de lire les données de l'image")?;
    
    if bytes.len() > MAX_IMAGE_SIZE {
        anyhow::bail!("L'image dépasse la taille maximale autorisée ({})", MAX_IMAGE_SIZE);
    }

    let image = image::load_from_memory(&bytes).context("Impossible de décoder l'image")?;
    Ok(image)
}

pub fn spawn_image_download(client: reqwest::blocking::Client, url: Arc<str>, sender: Sender<Option<DynamicImage>>) {
    std::thread::spawn(move || {
        let result = download_image(&client, &url).ok();
        let _ = sender.send(result);
    });
}

#[derive(Debug, Clone)]
pub enum ImageInner {
    Definition(ImageDefinition),
    Cached(Arc<DynamicImage>),
}

#[derive(Debug, Clone)]
pub struct Image {
    inner: ImageInner,
    alt: Option<Arc<str>>,
}

impl Image {
    pub fn get(&self, client: &reqwest::blocking::Client, (sender, receiver): (&Sender<Option<DynamicImage>>, &Receiver<Option<DynamicImage>>), root: &Path) -> Option<Arc<DynamicImage>> {
        match &self.inner {
            ImageInner::Definition(def) => def.decode(client, (sender, receiver), root).map(Arc::new),
            ImageInner::Cached(image) => Some(image.clone()),
        }
    }
    pub fn get_or_decode(&mut self, client: &reqwest::blocking::Client, (sender, receiver): (&Sender<Option<DynamicImage>>, &Receiver<Option<DynamicImage>>), root: &Path) -> Option<Arc<DynamicImage>> {
        match &mut self.inner {
            ImageInner::Definition(def) => {
                if let Some(image) = def.decode(client, (sender, receiver), root) {
                    let arc_image = Arc::new(image);
                    self.inner = ImageInner::Cached(arc_image.clone());
                    Some(arc_image)
                } else {
                    None
                }
            },
            ImageInner::Cached(image) => Some(image.clone()),
        }
    }
    pub fn get_alt(&self) -> Option<Arc<str>> {
        self.alt.clone()
    }
}

// TODO unload images if they take too much space/are unused. Keep images from url in cache.

#[derive(Debug)]
pub struct ImageRegistry {
    images: HashMap<Arc<str>, Image>,
    sender: Sender<Option<DynamicImage>>,
    receiver: Receiver<Option<DynamicImage>>,
    root: PathBuf,
}

impl Clone for ImageRegistry {
    fn clone(&self) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            images: self.images.clone(),
            sender,
            receiver,
            root: self.root.clone(),
        }
    }
}

impl ImageRegistry {
    pub fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            images: HashMap::new(),
            sender,
            receiver,
            root: PathBuf::from("."),
        }
    }
    pub fn from_json(data: Value) -> Result<Self> {
        let mut registry = Self::new();

        let Value::Object(map) = data else {
            anyhow::bail!("Le champ `images` doit être un objet");
        };
        for (key, value) in map {
            if let Value::Object(img_data) = value {
                let kind = img_data
                    .get("type")
                    .and_then(Value::as_str)
                    .with_context(|| format!("Champ `type` manquant ou invalide pour la clé {key}"))?;
                let alt: Option<Arc<str>> = img_data
                    .get("alt")
                    .and_then(Value::as_str)
                    .map(Arc::from);
                let inner = match kind {
                    "base64" => {
                        let data = img_data
                            .get("donnees")
                            .and_then(Value::as_str)
                            .with_context(|| {
                                format!("Champ `donnees` manquant ou invalide pour la clé {key}")
                            })?;
                        let format = img_data
                            .get("format")
                            .and_then(Value::as_str)
                            .with_context(|| {
                                format!("Champ `format` manquant ou invalide pour la clé {key}")
                            })?;
                        if !SUPPORTED_FORMATS.contains(&format) {
                            anyhow::bail!("Format d'image non supporté pour la clé {key}");
                        }
                        ImageInner::Definition(ImageDefinition::Base64 {
                            data: data.into(),
                            format: format.into(),
                        })
                    }
                    "url" => {
                        let url = img_data
                            .get("url")
                            .and_then(Value::as_str)
                            .with_context(|| {
                                format!("Champ `url` manquant ou invalide pour la clé {key}")
                            })?;
                        ImageInner::Definition(ImageDefinition::Url(url.into()))
                    }
                    "fichier" => {
                        let path = img_data
                            .get("chemin")
                            .and_then(Value::as_str)
                            .with_context(|| {
                                format!("Champ `chemin` manquant ou invalide pour la clé {key}")
                            })?;
                        ImageInner::Definition(ImageDefinition::Path(path.into()))
                    }
                    other => anyhow::bail!("Type d'image inconnu ({other}) pour la clé {key}"),
                };
                registry.images.insert(Arc::from(key), Image { inner, alt });
            } else {
                anyhow::bail!("Format d'image invalide (attend un objet) pour la clé {}", key);
            }
        }

        Ok(registry)
    }
    pub fn with_root(mut self, root: &Path) -> Self {
        self.root = root.to_path_buf();
        self
    }

    pub fn get(&self, key: &str, client: &reqwest::blocking::Client) -> Option<ImageResult> {
        let image = self.images.get(key)?;
        let res = image.get(client, (&self.sender, &self.receiver), &self.root);
        match res {
            Some(img) => Some(ImageResult::Defined(img)),
            None => Some(ImageResult::Alt(image.get_alt())),
        }
    }

    /// Returns whether an image definition exists for `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.images.contains_key(key)
    }

    /// Returns the number of registered image definitions.
    pub fn len(&self) -> usize {
        self.images.len()
    }

    /// Returns whether the registry contains no image definitions.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    pub fn get_and_cache(&mut self, key: &str, client: &reqwest::blocking::Client) -> Option<ImageResult> {
        let image = self.images.get_mut(key)?;
        let res = image.get_or_decode(client, (&self.sender, &self.receiver), &self.root);
        match res {
            Some(img) => Some(ImageResult::Defined(img)),
            None => Some(ImageResult::Alt(image.get_alt())),
        }
    }
}

pub enum ImageResult {
    Defined(Arc<DynamicImage>),
    Alt(Option<Arc<str>>),
}

pub fn fit_in_terminal(
    image: &DynamicImage,
    max_width: u16,
    max_height: u16,
) -> (u32, u32) {
    let image_width = image.width() as f64;
    let image_height = image.height() as f64;

    let max_width = max_width.max(1) as f64;
    let max_height = max_height.max(1) as f64;

    // Viuer models a terminal cell as approximately two pixels tall.
    let scale = (max_width / image_width)
        .min((max_height * 2.0) / image_height)
        .min(1.0);

    let width = (image_width * scale).round().max(1.0) as u32;
    let height = (image_height * scale / 2.0).ceil().max(1.0) as u32;

    (width, height)
}
