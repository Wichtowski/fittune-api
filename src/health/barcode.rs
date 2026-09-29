//! Retail barcodes (EAN-8, UPC-A, EAN-13, GTIN-14), checked and stored in one form so a product
//! scanned as UPC-A and as EAN-13 is the same product

/// The canonical form of `code`: digits only, UPC-A widened to EAN-13 with a leading zero.
/// `None` when it is not a retail barcode or its check digit is wrong
pub fn normalize(code: &str) -> Option<String> {
    let code = code.trim();
    if !matches!(code.len(), 8 | 12 | 13 | 14) || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits: Vec<u32> = code.bytes().map(|b| u32::from(b - b'0')).collect();
    let (check, body) = digits.split_last()?;
    // GS1: weights 3 and 1 alternate from the digit next to the check digit
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| if i % 2 == 0 { d * 3 } else { *d })
        .sum();
    if (10 - sum % 10) % 10 != *check {
        return None;
    }
    Some(if code.len() == 12 {
        format!("0{code}")
    } else {
        code.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn accepts_valid_ean13() {
        assert_eq!(normalize("5900259127761").as_deref(), Some("5900259127761"));
        assert_eq!(
            normalize(" 4000417025005 ").as_deref(),
            Some("4000417025005")
        );
    }

    #[test]
    fn widens_upc_a_to_ean13() {
        assert_eq!(normalize("036000291452").as_deref(), Some("0036000291452"));
    }

    #[test]
    fn accepts_ean8_and_gtin14() {
        assert_eq!(normalize("96385074").as_deref(), Some("96385074"));
        assert_eq!(
            normalize("15900259127768").as_deref(),
            Some("15900259127768")
        );
    }

    #[test]
    fn rejects_bad_check_digits_lengths_and_letters() {
        assert_eq!(normalize("5900259127762"), None);
        assert_eq!(normalize("12345"), None);
        assert_eq!(normalize("59002591277a1"), None);
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("123456789012345"), None);
    }
}
