//! Number formatting like msvcr70's _vsnprintf (shared by the VM and the assets).

/// "%f" of _vsnprintf (msvcr70, _cftof): the double to 21 significant digits ($I10_OUTPUT),
/// then rounded at the sixth decimal by adding 1 if the next digit is ≥ 5.
pub fn printf_f(f: f32) -> String {
    printf_f_prec(f, 6)
}

/// "%.Nf" of _vsnprintf: like printf_f, with `prec` decimals (prec ≥ 1).
pub fn printf_f_prec(f: f32, prec: i32) -> String {
    let x = f as f64;
    if x.is_infinite() {
        return if x < 0.0 { "-1.#INF00".into() } else { "1.#INF00".into() };
    }
    if x.is_nan() {
        // $I10_OUTPUT: "-1.#IND" only for the indefinite value (sign, quiet bit, zero payload); a
        // negative NaN with a payload (fld quiets a signalling NaN) is "-1.#QNAN".
        let bits = f.to_bits();
        let undefined = bits >> 31 != 0 && (bits & 0x7FFFFF) | 0x400000 == 0x400000;
        return if undefined {
            "-1.#IND00".into()
        } else if bits >> 31 != 0 {
            "-1.#QNAN0".into()
        } else {
            "1.#QNAN0".into()
        };
    }
    // -0 is written "0.000000" (msvcr70), a negative value rounded to zero keeps its sign.
    let sign = if x.is_sign_negative() && x != 0.0 { "-" } else { "" };
    // msvcr70's _cvt: at most 17 significant digits, then zeros (e.g. FloatToString of 3.4e38).
    let sci = format!("{:.16e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let mut digits: Vec<u8> = mantissa.bytes().filter(|b| b.is_ascii_digit()).map(|b| b - b'0').collect();
    let mut decpt: i32 = exp.parse::<i32>().unwrap_or(0) + 1;
    if x == 0.0 {
        decpt = 1;
    }
    // Digits kept: the integer part and `prec` decimals.
    let keep = decpt + prec;
    if keep < 0 {
        digits.clear();
    } else if (keep as usize) < digits.len() {
        let up = digits[keep as usize] >= 5;
        digits.truncate(keep as usize);
        if up {
            let mut i = digits.len();
            loop {
                if i == 0 {
                    digits.insert(0, 1);
                    decpt += 1;
                    break;
                }
                i -= 1;
                if digits[i] == 9 {
                    digits[i] = 0;
                } else {
                    digits[i] += 1;
                    break;
                }
            }
        }
    }
    let digit = |i: i32| if i >= 0 && (i as usize) < digits.len() { (b'0' + digits[i as usize]) as char } else { '0' };
    let mut out = String::from(sign);
    if decpt <= 0 {
        out.push('0');
    } else {
        (0..decpt).for_each(|i| out.push(digit(i)));
    }
    out.push('.');
    (0..prec).for_each(|k| out.push(digit(decpt + k)));
    out
}
