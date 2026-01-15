use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::Cell,
};

pub fn fuzzy_cell(
    display_value: &str,
    query: &str,
    matcher: &SkimMatcherV2,
    base_style: Style,
    match_style: Style,
) -> Cell<'static> {
    if query.is_empty() {
        return Cell::from(format!(" {}", display_value)).style(base_style);
    }

    let Some((_score, indices)) = matcher.fuzzy_indices(display_value, query) else {
        return Cell::from(format!(" {}", display_value)).style(base_style);
    };

    if indices.is_empty() {
        return Cell::from(format!(" {}", display_value)).style(base_style);
    }

    let mut spans = vec![Span::styled(" ", base_style)];
    let mut current_segment = String::new();
    let mut is_matching_segment = false;
    let matched_char_indices: std::collections::HashSet<usize> = indices.into_iter().collect();

    for (char_idx, char) in display_value.chars().enumerate() {
        let is_current_char_a_match = matched_char_indices.contains(&char_idx);

        if char_idx == 0 {
            is_matching_segment = is_current_char_a_match;
        }

        if is_current_char_a_match != is_matching_segment {
            let style = if is_matching_segment {
                match_style
            } else {
                base_style
            };
            spans.push(Span::styled(current_segment, style));
            current_segment = String::new();
            is_matching_segment = is_current_char_a_match;
        }

        current_segment.push(char);
    }

    if !current_segment.is_empty() {
        let style = if is_matching_segment {
            match_style
        } else {
            base_style
        };
        spans.push(Span::styled(current_segment, style));
    }

    Cell::from(Line::from(spans))
}
