use base64::{engine::general_purpose::STANDARD, Engine as _};

use super::MAX_REFERENCE_CARD_BYTES;

/// Build the designer instructions. Pure so tests can pin the contract:
/// style-match-the-avatar is DEFAULT behavior; owner directions (art AND
/// card text) take primacy over those style defaults, but never over the
/// fixed contract (frame identity, geometry, text fidelity).
pub(crate) fn build_card_instructions(
    agent_name: &str,
    persona_notes: &str,
    style_notes: &str,
) -> String {
    let owner_directions = if style_notes.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\nOWNER'S DIRECTIONS — these override the default art-style and copy guidance \
             below wherever they conflict (they cannot change the frame, layout, or \
             text-fidelity requirements). The owner may direct the art, the card text \
             (type line, ability, flavor), or both:\n{style_notes}\n"
        )
    };
    format!(
        r#"You are designing one premium collectible trading card for the Buzz agent "{agent_name}".

Input image 1 is the official Buzz card template: a rounded 2:3 portrait card with an iridescent foil perimeter, a white inner panel, Buzz Agent branding and card number at the top, a large circular art window, a bold name and subtitle below it, and two compact metadata columns at the bottom. Input image 2 is the agent's avatar — study its exact art style: medium, pixel grid if any, palette, shading, and background motifs.

Persona notes for the card copy:
{persona_notes}
{owner_directions}
First, write concise card copy that describes this agent:
- a short subtitle beneath the agent name,
- one compact "Good for" phrase,
- one compact "Vibes" phrase.
Where the owner's directions specify card text, use their wording within the 220-character text-box limit below (edited only for spelling; if their text exceeds the limit, condense it minimally while keeping their words and intent); invent copy only for the parts they left open.
Keep the total generated card copy under 220 characters so it renders cleanly.

Then generate the finished card with the image tool, exactly 1024x1536 portrait:
- The frame and information architecture must follow input image 1 faithfully: keep its concentric rounded geometry, iridescent foil perimeter, white inner panel, top brand/number row, circular art mask, lower name block, and two-column metadata row.
- Default art style: match input image 2's art style EXACTLY — same medium, same pixel density if pixel art, palette, and shading. Replace the template's placeholder portrait with a custom interpretation of this agent inside the same circular art mask. The owner's directions above override the default art styling where they conflict.
- Use "{agent_name}" as the bold card name and set the short subtitle directly beneath it.
- Use the bottom two columns for "Good for" and "Vibes". Keep both phrases brief enough to fit without crowding.
- Derive the foil perimeter palette from the avatar unless the owner's directions explicitly request a different palette.
Render all text with perfect fidelity."#
    )
}

pub(crate) fn build_card_followup_instructions(
    agent_name: &str,
    persona_notes: &str,
    style_notes: &str,
) -> String {
    format!(
        "{}\n\nFOLLOW-UP REVISION: Input image 3 is the current finished card. Apply the \
         owner's directions as a revision to that card. Preserve every visual and textual \
         detail the owner did not ask to change, while continuing to obey the fixed Buzz \
         template, geometry, and text-fidelity requirements.",
        build_card_instructions(agent_name, persona_notes, style_notes)
    )
}

pub(crate) fn decode_reference_card(
    reference_card_png_base64: Option<&str>,
) -> Result<Option<Vec<u8>>, String> {
    let Some(encoded) = reference_card_png_base64.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    if encoded.len() > MAX_REFERENCE_CARD_BYTES.div_ceil(3) * 4 {
        return Err("The follow-up card reference is too large.".to_string());
    }
    let bytes = STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| "The follow-up card reference is not valid base64.".to_string())?;
    if bytes.len() > MAX_REFERENCE_CARD_BYTES {
        return Err("The follow-up card reference is too large.".to_string());
    }
    image::load_from_memory(&bytes)
        .map_err(|_| "The follow-up card reference is not a valid image.".to_string())?;
    Ok(Some(bytes))
}
