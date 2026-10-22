use anyhow::{Context, Result, bail};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::config::Config;

/// Words Whisper gave less than this probability are worth a second look.
const DOUBT_BELOW: f32 = 0.4;

pub struct Transcript {
    pub text: String,
    /// Words Whisper was unsure about, for the dictionary review.
    pub doubtful: Vec<String>,
}

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
    pub fn transcribe(&self, pcm: &[f32], glossary: &[&str]) -> Result<Transcript> {
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
        let eot = self.context.token_eot();
        let mut tokens = Vec::new();
        for segment in state.as_iter() {
            for index in 0..segment.n_tokens() {
                let Some(token) = segment.get_token(index) else {
                    continue;
                };
                // Ids from end-of-text up are special tokens, not text.
                if token.token_id() >= eot {
                    continue;
                }
                let bytes = token.to_bytes().unwrap_or_default().to_vec();
                tokens.push((bytes, token.token_probability()));
            }
        }
        Ok(Transcript {
            text: text.trim().to_string(),
            doubtful: doubtful_words(&tokens),
        })
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

/// Joins tokens into words, since a token that starts with a space starts a word, and keeps
/// the words whose least likely token is below `DOUBT_BELOW`.
fn doubtful_words(tokens: &[(Vec<u8>, f32)]) -> Vec<String> {
    let mut words: Vec<(Vec<u8>, f32)> = Vec::new();
    for (bytes, probability) in tokens {
        match words.last_mut() {
            Some((word, lowest)) if !bytes.starts_with(b" ") => {
                word.extend_from_slice(bytes);
                *lowest = lowest.min(*probability);
            }
            _ => words.push((bytes.clone(), *probability)),
        }
    }
    let mut doubtful: Vec<String> = Vec::new();
    for (bytes, lowest) in words {
        let word = String::from_utf8_lossy(&bytes);
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        if lowest < DOUBT_BELOW
            && word.chars().any(char::is_alphabetic)
            && !doubtful
                .iter()
                .any(|known| known.eq_ignore_ascii_case(word))
        {
            doubtful.push(word.to_string());
        }
    }
    doubtful
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
    fn doubtful_words_join_tokens_and_use_the_lowest_probability() {
        let token = |text: &str, p: f32| (text.as_bytes().to_vec(), p);
        let tokens = [
            token(" O", 0.99),
            token(" Bit", 0.9),
            token("forge", 0.2),
            token(",", 0.1),
            token(" bitforge", 0.3),
            token(" é", 0.95),
        ];
        assert_eq!(doubtful_words(&tokens), ["Bitforge"]);
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
