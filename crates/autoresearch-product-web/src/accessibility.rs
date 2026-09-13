//! Bounded accessibility probes; diagnostics, not WCAG conformance claims.

use crate::browser::BrowserError;
use headless_chrome::Tab;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const MAX_KEYBOARD_STEPS: u32 = 8;

/// Evidence from one rendered viewport. Automated sampling is incomplete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityEvidence {
    /// Visible native or declared interactive controls counted before traversal.
    pub focusable_count: u32,
    /// Selectors reached by Tab, in browser-observed order (bounded to eight).
    pub tab_targets: Vec<String>,
    /// Tab stops with detectable focus-visible outline or box shadow.
    pub visible_focus_count: u32,
    /// Sampled text selectors below WCAG 2.x contrast ratio for size/weight.
    pub contrast_failures: Vec<String>,
    /// Text nodes whose contrast could not be computed safely.
    pub contrast_unavailable_count: u32,
}

impl AccessibilityEvidence {
    /// Bounded keyboard/focus status: pass, fail, or unavailable.
    #[must_use]
    pub fn keyboard_status(&self) -> &'static str {
        if self.focusable_count == 0 || self.focusable_count > MAX_KEYBOARD_STEPS {
            "unavailable"
        } else if self.tab_targets.len() != self.focusable_count as usize
            || self.visible_focus_count != self.focusable_count
        {
            "fail"
        } else {
            "pass"
        }
    }

    /// Sampled solid-colour contrast status: pass, fail, or unavailable.
    #[must_use]
    pub fn contrast_status(&self) -> &'static str {
        if !self.contrast_failures.is_empty() {
            "fail"
        } else if self.contrast_unavailable_count > 0 {
            "unavailable"
        } else {
            "pass"
        }
    }
}

#[derive(Deserialize)]
struct FocusSnapshot {
    target: String,
    visible: bool,
}

#[derive(Deserialize)]
struct ContrastSnapshot {
    failures: Vec<String>,
    unavailable: u32,
}

pub(crate) fn inspect(tab: &Tab) -> Result<AccessibilityEvidence, BrowserError> {
    let focusable_count = tab
        .evaluate(FOCUSABLE_COUNT_SCRIPT, false)
        .map_err(|_| BrowserError::Inspection)?
        .value
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(BrowserError::Inspection)?;
    let mut tab_targets = Vec::new();
    let mut visible_focus_count = 0;
    for _ in 0..focusable_count.min(MAX_KEYBOARD_STEPS) {
        tab.press_key("Tab").map_err(|_| BrowserError::Inspection)?;
        let focus: FocusSnapshot = evaluate_json(tab, FOCUS_SNAPSHOT_SCRIPT)?;
        if focus.target == "body" || tab_targets.contains(&focus.target) {
            break;
        }
        visible_focus_count += u32::from(focus.visible);
        tab_targets.push(focus.target);
    }
    let contrast: ContrastSnapshot = evaluate_json(tab, CONTRAST_SCRIPT)?;
    Ok(AccessibilityEvidence {
        focusable_count,
        tab_targets,
        visible_focus_count,
        contrast_failures: contrast.failures,
        contrast_unavailable_count: contrast.unavailable,
    })
}

fn evaluate_json<T: DeserializeOwned>(tab: &Tab, script: &str) -> Result<T, BrowserError> {
    let result = tab
        .evaluate(script, false)
        .map_err(|_| BrowserError::Inspection)?;
    let serialized = result
        .value
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(BrowserError::Inspection)?;
    serde_json::from_str(&serialized).map_err(|_| BrowserError::Inspection)
}

const FOCUSABLE_COUNT_SCRIPT: &str = r"Array.from(document.querySelectorAll('a[href],button,input:not([type=hidden]),select,textarea,[tabindex]')).filter(el => {
  const style = getComputedStyle(el);
  const rect = el.getBoundingClientRect();
  return !el.disabled && el.tabIndex >= 0 && style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0;
}).length";

const FOCUS_SNAPSHOT_SCRIPT: &str = r#"JSON.stringify((() => {
  const el = document.activeElement;
  const style = getComputedStyle(el);
  const outline = style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) > 0;
  const shadow = style.boxShadow !== 'none';
  return {
    target: el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + (el.getAttribute('href') ? '[href="' + el.getAttribute('href') + '"]' : ''),
    visible: el.matches(':focus-visible') && (outline || shadow)
  };
})())"#;

const CONTRAST_SCRIPT: &str = r"JSON.stringify((() => {
  const failures = [];
  let unavailable = 0;
  const parse = (color) => {
    const match = color.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)(?:\s*,\s*([\d.]+))?\s*\)$/);
    if (!match || (match[4] !== undefined && Number(match[4]) < 1)) return null;
    return [Number(match[1]), Number(match[2]), Number(match[3])];
  };
  const luminance = (rgb) => rgb.map(value => {
    const channel = value / 255;
    return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
  }).reduce((sum, value, index) => sum + value * [.2126, .7152, .0722][index], 0);
  const background = (el) => {
    for (let node = el; node; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.backgroundImage !== 'none') return null;
      const color = parse(style.backgroundColor);
      if (color) return color;
    }
    return [255, 255, 255];
  };
  const nodes = Array.from(document.querySelectorAll('body *')).filter(el =>
    el.children.length === 0 && el.textContent.trim() &&
    !['SCRIPT', 'STYLE', 'NOSCRIPT'].includes(el.tagName) &&
    getComputedStyle(el).display !== 'none' &&
    getComputedStyle(el).visibility !== 'hidden' &&
    el.getBoundingClientRect().width > 0
  ).slice(0, 200);
  if (!nodes.length) unavailable++;
  for (const el of nodes) {
    const style = getComputedStyle(el);
    const foreground = parse(style.color);
    const bg = background(el);
    if (!foreground || !bg) { unavailable++; continue; }
    const light = Math.max(luminance(foreground), luminance(bg));
    const dark = Math.min(luminance(foreground), luminance(bg));
    const ratio = (light + .05) / (dark + .05);
    const size = parseFloat(style.fontSize);
    const weight = style.fontWeight === 'bold' ? 700 : parseInt(style.fontWeight, 10);
    const large = size >= 24 || (size >= 18.66 && weight >= 700);
    if (ratio + .001 < (large ? 3 : 4.5) && failures.length < 20) {
      failures.push(el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + ' ratio=' + ratio.toFixed(2));
    }
  }
  return { failures, unavailable };
})())";

#[cfg(test)]
mod tests {
    use super::AccessibilityEvidence;

    #[test]
    fn unmeasured_rules_are_unavailable_not_passed() {
        let evidence = AccessibilityEvidence {
            focusable_count: 0,
            tab_targets: vec![],
            visible_focus_count: 0,
            contrast_failures: vec![],
            contrast_unavailable_count: 1,
        };
        assert_eq!(evidence.keyboard_status(), "unavailable");
        assert_eq!(evidence.contrast_status(), "unavailable");
    }
}
