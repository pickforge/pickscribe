use anyhow::{Context, Result, bail};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::config::Config;

/// Keeps the Whisper model loaded on the GPU between recordings.
pub struct Transcriber {
    context: WhisperContext,
    languages: Vec<String>,
    threads: usize,
}

impl Transcriber {
    pub fn load(config: &Config) -> Result<Self> {
        if !config.whisper_model.is_file() {
            bail!(
                "Whisper model not found at {}",
                config.whisper_model.display()
            );
        }
        for language in &config.languages {
            if whisper_rs::get_lang_id(language).is_none() {
                bail!("unknown language code {language:?} in languages");
            }
        }
        // Route whisper.cpp logs into the `log` facade, which stays silent here.
        whisper_rs::install_logging_hooks();
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);
        let context = WhisperContext::new_with_params(&config.whisper_model, params)
            .with_context(|| format!("cannot load {}", config.whisper_model.display()))?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        Ok(Self {
            context,
            languages: config.languages.clone(),
            threads,
        })
    }

    /// Transcribes 16 kHz mono audio, spelling the glossary terms as given.
    pub fn transcribe(&self, pcm: &[f32], glossary: &[&str]) -> Result<String> {
        let mut state = self.context.create_state()?;
        let language = self.pick_language(&mut state, pcm)?;
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_language(Some(language));
        let prompt = vocabulary_prompt(glossary);
        if !prompt.is_empty() {
            params.set_initial_prompt(&prompt);
        }
        params.set_n_threads(self.threads as i32);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        state.full(params, pcm)?;
        let text: String = state
            .as_iter()
            .map(|segment| segment.to_str_lossy().map(|s| s.into_owned()))
            .collect::<Result<_, _>>()?;
        Ok(text.trim().to_string())
    }

    /// Picks the most likely of the configured languages. Free detection over all languages
    /// mislabels short Portuguese clips as Spanish or Galician too often.
    fn pick_language(&self, state: &mut WhisperState, pcm: &[f32]) -> Result<&str> {
        if let [only] = self.languages.as_slice() {
            return Ok(only);
        }
        state.pcm_to_mel(pcm, self.threads)?;
        let (_, probabilities) = state.lang_detect(0, self.threads)?;
        let score = |language: &str| {
            whisper_rs::get_lang_id(language)
                .and_then(|id| probabilities.get(usize::try_from(id).ok()?).copied())
                .unwrap_or(0.0)
        };
        Ok(best_language(&self.languages, score))
    }
}

fn best_language(languages: &[String], score: impl Fn(&str) -> f32) -> &str {
    languages
        .iter()
        .max_by(|a, b| score(a).total_cmp(&score(b)))
        .map_or("auto", String::as_str)
}

/// Whisper copies the spelling it sees in the initial prompt.
fn vocabulary_prompt(vocabulary: &[&str]) -> String {
    if vocabulary.is_empty() {
        return String::new();
    }
    format!("Glossary: {}.", vocabulary.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_language_picks_the_highest_configured_score() {
        let languages = vec!["pt".to_string(), "en".to_string()];
        let score = |language: &str| match language {
            "pt" => 0.2,
            "en" => 0.7,
            _ => 0.0,
        };
        assert_eq!(best_language(&languages, score), "en");
        assert_eq!(best_language(&[], score), "auto");
    }

    #[test]
    fn vocabulary_becomes_a_glossary_prompt() {
        assert_eq!(vocabulary_prompt(&[]), "");
        assert_eq!(
            vocabulary_prompt(&["Pickforge", "Prova Clara"]),
            "Glossary: Pickforge, Prova Clara."
        );
    }
}
