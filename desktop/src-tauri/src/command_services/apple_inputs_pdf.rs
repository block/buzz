use std::path::Path;

use super::{read_apple_inputs_blocking, AppleInputRequest, AppleInputResponse, PdfArguments};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlanningPdfPage {
    pub(crate) page: usize,
    pub(crate) text: String,
    pub(crate) confidence: Option<f64>,
}

pub(crate) fn extract_planning_pdf(path: &Path) -> Result<Vec<PlanningPdfPage>, String> {
    let response = read_apple_inputs_blocking(AppleInputRequest::ExtractPdf(PdfArguments {
        path: path
            .to_str()
            .ok_or_else(|| "selected PDF path is not valid UTF-8".to_string())?
            .to_string(),
    }));
    parse_pdf_response(&response)
}

fn parse_pdf_response(response: &AppleInputResponse) -> Result<Vec<PlanningPdfPage>, String> {
    if let Some(error) = response.error() {
        return Err(error.to_string());
    }
    response
        .records()
        .iter()
        .map(|record| {
            let page = record
                .fields()
                .get("page")
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|page| *page > 0)
                .ok_or_else(|| "PDF helper returned an invalid page number".to_string())?;
            let text = record
                .fields()
                .get("text")
                .cloned()
                .ok_or_else(|| "PDF helper omitted page text".to_string())?;
            let confidence = record
                .fields()
                .get("confidence")
                .map(|value| {
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|value| (0.0..=1.0).contains(value))
                        .ok_or_else(|| "PDF helper returned invalid OCR confidence".to_string())
                })
                .transpose()?;
            Ok(PlanningPdfPage {
                page,
                text,
                confidence,
            })
        })
        .collect()
}
