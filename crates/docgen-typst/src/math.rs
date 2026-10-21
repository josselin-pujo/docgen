//! LaTeX (subset) → typst math translation.
//!
//! docgen math sources are authored in the LaTeX-ish syntax every markdown
//! writer knows (`$…$`, `$$…$$`). Typst math is *not* LaTeX, so the source
//! is translated here, command by command, into typst math markup. The
//! supported surface is the common notation corpus — Greek letters, accented
//! and blackboard/cal/bold styles, fractions, roots, sub/superscripts,
//! sums/integrals/limits, matrices, cases, aligned multi-line equations,
//! `\left(…\right)`, and spacing macros.
//!
//! Every typst symbol NAME emitted here was verified against the embedded
//! typst 0.15 (an unknown name fails the whole compile); symbols without a
//! verified typst name are emitted as unicode literals, which always
//! compile and render through the math font. Anything else unknown degrades
//! visibly (the command name as upright text) rather than failing the
//! render — an unknown construct must never break the document.

/// Translates the *inner* text of a math span (delimiters already stripped
/// by the markdown parser) to typst math markup.
pub fn translate(src: &str) -> String {
    translate_inner(src, false)
}

/// Display-mode translation: big operators (`\sum`, `\prod`, `\lim`, …)
/// get under/over scripts via typst's `limits()`, matching LaTeX `$$…$$`
/// conventions (typst 0.15 never switches to display layout on its own).
pub fn translate_display(src: &str) -> String {
    translate_inner(src, true)
}

fn translate_inner(src: &str, display: bool) -> String {
    let toks = tokenize(src);
    let mut t = Translator {
        toks: &toks,
        pos: 0,
        last: Last::None,
        word_mode: false,
        display,
    };
    t.sequence(Stop::End).trim().to_string()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Cmd(String),
    OpenBrace,
    CloseBrace,
    Sub,
    Sup,
    Amp,
    RowBreak, // `\\`
    Ch(char),
    Space,
}

/// What was just emitted — decides how the next piece is spaced.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Last {
    None,
    /// A bare letter or a command expansion's `)`: a following `(` is
    /// juxtaposition (spaced). A following letter must also be spaced —
    /// typst resolves multi-letter words as code variables and FAILS the
    /// compile on unknown ones (`4ac` → "unknown variable: ac").
    Atom,
    /// A digit: digit runs glue (`42`), letters start a new factor.
    Digit,
    /// A function name (`sin`, `log`, …): a following `(` IS its call.
    Fn,
    /// An opening paren was just emitted — the next atom glues to it.
    Open,
    /// An operator in unary position (`(-x`, `= -b`): the operand glues.
    OpUnary,
    /// An operator in binary position (`a + b`): spaces on both sides.
    OpBinary,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stop {
    End,
    /// Stop at (and consume) the closing `}`.
    Brace,
}

struct Translator<'a> {
    toks: &'a [Tok],
    pos: usize,
    last: Last,
    /// Inside a word argument (environment name, `\text{…}`, `\mathbb{…}`):
    /// letter runs glue into words instead of staying separate factors.
    word_mode: bool,
    /// Display mode: big operators take under/over scripts.
    display: bool,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                // Row break.
                Some('\\') => toks.push(Tok::RowBreak),
                // Escaped punctuation (`\,` `\;` `\{` `\}` …) and the explicit
                // space `\ ` — all carried as one-char commands.
                Some(n) if !n.is_ascii_alphabetic() => toks.push(Tok::Cmd(n.to_string())),
                Some(n) => {
                    let mut name = String::from(n);
                    while chars.peek().is_some_and(|&x| x.is_ascii_alphabetic()) {
                        name.push(chars.next().unwrap());
                    }
                    toks.push(Tok::Cmd(name));
                    // The whitespace after a word command is its terminator,
                    // not a rendered space — but preserve ONE for the
                    // argument spacing (`\sin x` → `sin x`).
                    if chars.peek().is_some_and(|x| x.is_whitespace()) {
                        while chars.peek().is_some_and(|x| x.is_whitespace()) {
                            chars.next();
                        }
                        toks.push(Tok::Space);
                    }
                }
                None => toks.push(Tok::Cmd("\\".to_string())),
            },
            '{' => toks.push(Tok::OpenBrace),
            '}' => toks.push(Tok::CloseBrace),
            '_' => toks.push(Tok::Sub),
            '^' => toks.push(Tok::Sup),
            '&' => toks.push(Tok::Amp),
            '\n' | '\r' => {
                while chars.peek().is_some_and(|x| x.is_whitespace()) {
                    chars.next();
                }
                toks.push(Tok::Space);
            }
            c if c.is_whitespace() => {
                while chars.peek().is_some_and(|x| x.is_whitespace()) {
                    chars.next();
                }
                toks.push(Tok::Space);
            }
            c => toks.push(Tok::Ch(c)),
        }
    }
    toks
}

impl<'a> Translator<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(Tok::Space)) {
            self.next();
        }
    }

    /// Translates until `stop`. Consumes the stop marker for [`Stop::Brace`].
    fn sequence(&mut self, stop: Stop) -> String {
        let mut out = String::new();
        while let Some(tok) = self.peek().cloned() {
            if tok == Tok::CloseBrace && stop == Stop::Brace {
                self.pos += 1;
                break;
            }
            self.pos += 1;
            // The spacing decision reads the state *before* this token —
            // `token()` advances it.
            let before = self.last;
            if let Some(piece) = self.token(tok) {
                if piece == " " {
                    if !out.is_empty() && !out.ends_with(' ') {
                        out.push(' ');
                    }
                } else {
                    let need = !out.is_empty()
                        && !out.ends_with(' ')
                        && !piece.starts_with([',', ';', ')', ']', '^', '_', '&'])
                        && match piece.as_str() {
                            // After a function name, `(` is the argument
                            // list; after an atom it is juxtaposition
                            // (space); right after an opener it glues.
                            // Word mode: letter runs are names, not products.
                            p if self.word_mode
                                && before == Last::Atom
                                && p.len() == 1
                                && p.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) =>
                            {
                                false
                            }
                            "(" => before == Last::Atom,
                            // Digits glue into number runs (`42`); letters
                            // always start a new factor (see `Last::Atom`).
                            p if p.len() == 1
                                && p.chars().next().is_some_and(|c| c.is_ascii_digit()) =>
                            {
                                before != Last::Digit && before != Last::Open
                            }
                            // Everything glues after an opener or a unary
                            // operator (`(-x`, `(-oo`).
                            _ => !matches!(before, Last::Open | Last::OpUnary),
                        };
                    if need {
                        out.push(' ');
                    }
                    out.push_str(&piece);
                }
            }
        }
        out
    }

    fn token(&mut self, tok: Tok) -> Option<String> {
        match tok {
            Tok::Space => {
                self.last = Last::None;
                Some(" ".to_string())
            }
            Tok::OpenBrace => {
                // Group content starts fresh: a leading `-` is unary.
                self.last = Last::Open;
                let inner = self.sequence(Stop::Brace);
                self.last = Last::Atom;
                Some(inner)
            }
            Tok::CloseBrace => None, // stray `}` outside any group: skipped
            Tok::Sub => self.script('_'),
            Tok::Sup => self.script('^'),
            Tok::Amp => {
                self.last = Last::Other;
                Some("&".to_string())
            }
            Tok::RowBreak => {
                self.last = Last::None;
                Some("\\\\".to_string())
            }
            Tok::Cmd(name) => self.command(&name),
            Tok::Ch(c) => Some(self.char(c)),
        }
    }

    /// `_…` / `^…` with a group or single-token argument → `_(…)` / `^(…)`
    /// (parenthesized scripts are grouping, never rendered glyphs).
    fn script(&mut self, mark: char) -> Option<String> {
        let arg = self.arg()?;
        // A completed script binds: what follows spaces, never glues.
        self.last = Last::Other;
        Some(format!("{mark}({arg})"))
    }

    /// One argument parsed in word mode (env names, `\text{…}`, …).
    fn word_arg(&mut self) -> Option<String> {
        self.word_mode = true;
        let out = self.arg();
        self.word_mode = false;
        out
    }

    /// A big operator: in display mode typst needs `limits()` for under/over
    /// scripts (verified: plain `sum_(…)^…` keeps scripts beside).
    fn big_op(&mut self, name: &str) -> String {
        self.last = Last::Other;
        if self.display {
            format!("limits({name})")
        } else {
            name.to_string()
        }
    }

    /// One LaTeX argument: a brace group, a single character, or a single
    /// command (`\frac ab`, `x^\alpha`).
    fn arg(&mut self) -> Option<String> {
        self.skip_spaces();
        match self.next()? {
            Tok::OpenBrace => {
                // Group content starts fresh: a leading `-` is unary.
                self.last = Last::Open;
                let inner = self.sequence(Stop::Brace);
                self.last = Last::Atom;
                Some(inner)
            }
            Tok::Ch(c) => {
                self.last = Last::Atom;
                Some(self.char(c))
            }
            Tok::Cmd(name) => self.command(&name),
            _ => Some(String::new()),
        }
    }

    fn char(&mut self, c: char) -> String {
        match c {
            '0'..='9' => {
                self.last = Last::Digit;
                c.to_string()
            }
            'a'..='z' | 'A'..='Z' => {
                self.last = Last::Atom;
                c.to_string()
            }
            '(' => {
                // Spacing (juxtaposition vs call vs nested) is decided by the
                // sequence loop from the *previous* state; the next atom
                // always glues to the paren.
                self.last = Last::Open;
                "(".to_string()
            }
            '+' | '-' | '=' | '<' | '>' | '~' => {
                // Unary (glued operand) right after an opener, another
                // operator, or at the start; binary (spaced) otherwise.
                self.last = match self.last {
                    Last::Open | Last::None | Last::OpUnary | Last::OpBinary => Last::OpUnary,
                    _ => Last::OpBinary,
                };
                c.to_string()
            }
            _ => {
                self.last = Last::Other;
                c.to_string()
            }
        }
    }

    fn command(&mut self, name: &str) -> Option<String> {
        Some(match name {
            // -- symbols with verified typst names ----------------------------
            // Full Greek set (both cases), operators, arrows.
            "alpha" | "beta" | "gamma" | "delta" | "epsilon" | "zeta" | "eta" | "theta"
            | "iota" | "kappa" | "lambda" | "mu" | "nu" | "xi" | "pi" | "rho" | "sigma" | "tau"
            | "upsilon" | "phi" | "chi" | "psi" | "omega" | "Gamma" | "Delta" | "Theta"
            | "Lambda" | "Xi" | "Pi" | "Sigma" | "Upsilon" | "Phi" | "Psi" | "Omega" | "nabla"
            | "times" | "approx" | "equiv" | "mapsto" | "emptyset" | "in" | "forall" | "exists"
            | "prime" | "bullet" | "star" | "integral" => name.to_string(),
            "infty" => "oo".into(),
            "int" => "integral".into(),
            "oint" => "∮".into(),
            // Big operators: display mode moves scripts under/over.
            "sum" => self.big_op("sum"),
            "prod" => self.big_op("product"),
            "coprod" => self.big_op("∐"),
            "cdot" => "dot.op".into(),
            "varepsilon" => "epsilon.alt".into(),
            "pm" => "±".into(),
            "mp" => "∓".into(),
            "leq" | "le" => "<=".into(),
            "geq" | "ge" => ">=".into(),
            "neq" | "ne" => "!=".into(),
            "to" | "rightarrow" => "->".into(),
            "leftarrow" | "gets" => "<-".into(),
            "leftrightarrow" => "<->".into(),
            "Rightarrow" | "implies" => "=>".into(),
            "partial" => "partial".into(),
            "top" => "top".into(),
            "bot" => "bot".into(),
            "notin" => "∉".into(),
            "subset" => "⊂".into(),
            "subseteq" => "subset.eq".into(),
            "supset" => "⊃".into(),
            "supseteq" => "supset.eq".into(),
            "cup" => "union".into(),
            "cap" => "∩".into(),
            "oplus" => "⊕".into(),
            "otimes" => "⊗".into(),
            "ldots" | "dots" | "dotsc" => "…".into(),
            "cdots" | "dotsb" => "⋯".into(),
            "angle" => "∠".into(),
            "propto" => "∝".into(),
            "circ" => "∘".into(),
            "varnothing" => "emptyset".into(),
            "vartheta" => "ϑ".into(),
            "varphi" => "φ".into(),
            "varpi" => "ϖ".into(),
            "varrho" => "ϱ".into(),
            "varsigma" => "ς".into(),

            // -- spacing (typst's predefined math spacings, verified) ---------
            "quad" => "quad".into(),
            "qquad" => "quad quad".into(),
            "," => "thin".into(),
            ";" => "thick".into(),
            ":" => "med".into(),
            "!" => String::new(), // negative space: no typst equivalent, drop
            " " => "med".into(),

            // -- argument-taking ----------------------------------------------
            "frac" | "dfrac" | "tfrac" => {
                let a = self.arg()?;
                let b = self.arg()?;
                self.last = Last::Atom;
                format!("frac({a}, {b})")
            }
            "sqrt" => {
                self.skip_spaces();
                let mut index = None;
                if matches!(self.peek(), Some(Tok::Ch('['))) {
                    self.next();
                    let mut idx = String::new();
                    loop {
                        match self.next() {
                            Some(Tok::Ch(']')) | None => break,
                            Some(t) => {
                                if let Some(piece) = self.token(t) {
                                    idx.push_str(&piece);
                                }
                            }
                        }
                    }
                    index = Some(idx);
                }
                let body = self.arg()?;
                self.last = Last::Atom;
                match index {
                    // typst's nth root is `root(index, body)`.
                    Some(idx) => format!("root({idx}, {body})"),
                    None => format!("sqrt({body})"),
                }
            }
            "binom" => {
                let a = self.arg()?;
                let b = self.arg()?;
                self.last = Last::Atom;
                format!("binom({a}, {b})")
            }
            "text" | "mathrm" | "operatorname" | "textrm" => {
                let body = self.word_arg()?;
                self.last = Last::Other;
                format!("\"{}\"", body.replace('"', "\\\""))
            }
            "mathbf" | "bm" | "boldsymbol" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("bold({body})")
            }
            "mathcal" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("cal({body})")
            }
            "mathbb" => {
                let body = self.word_arg()?;
                self.last = Last::Other;
                blackboard(&body)
            }
            "hat" | "widehat" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("hat({body})")
            }
            "bar" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("macron({body})")
            }
            "overline" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("overline({body})")
            }
            "underline" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("underline({body})")
            }
            "tilde" | "widetilde" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("tilde({body})")
            }
            "dot" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("dot({body})")
            }
            "ddot" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("dot.double({body})")
            }
            "vec" => {
                let body = self.arg()?;
                self.last = Last::Atom;
                format!("arrow({body})")
            }
            "left" => {
                self.skip_spaces();
                self.last = Last::None;
                match self.next() {
                    // `\left( … \right)` → `lr(( … ))`: the doubled parens
                    // are the scaled delimiters, the extra paren comes back
                    // from the matching `\right)`.
                    Some(Tok::Ch('(')) => "lr((".to_string(),
                    Some(Tok::Ch('[')) => "lr(bracket.l ".to_string(),
                    // No verified brace delimiter name: literal braces.
                    Some(Tok::Ch('{')) => "\"{\" ".to_string(),
                    Some(Tok::Cmd(c)) => {
                        format!("lr({} ", self.command(&c).unwrap_or_default())
                    }
                    _ => "lr(".to_string(),
                }
            }
            "right" => {
                self.skip_spaces();
                self.last = Last::Atom;
                match self.next() {
                    Some(Tok::Ch(')')) => "))".to_string(),
                    Some(Tok::Ch(']')) => "bracket.r )".to_string(),
                    Some(Tok::Ch('}')) => " \"}\"".to_string(),
                    _ => ")".to_string(),
                }
            }

            // -- functions (typst predefines the upright operator) ------------
            "sin" | "cos" | "tan" | "log" | "ln" | "exp" | "det" | "max" | "min" | "gcd"
            | "deg" | "dim" | "mod" | "csc" | "sec" | "cot" | "arg" => {
                self.last = Last::Fn;
                name.to_string()
            }
            "lim" => {
                // Display-mode limit puts the bound underneath (LaTeX $$…$$
                // convention), like the other big operators.
                if self.display {
                    self.last = Last::Other;
                    "limits(lim)".to_string()
                } else {
                    self.last = Last::Fn;
                    "lim".to_string()
                }
            }

            // -- no-ops ---------------------------------------------------------
            "limits" | "nolimits" | "displaystyle" | "textstyle" | "big" | "Big" | "bigg"
            | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "left." | "right." => String::new(),

            // -- environments -----------------------------------------------------
            "begin" => {
                let env = self.word_arg()?;
                match env.as_str() {
                    "pmatrix" | "bmatrix" | "vmatrix" | "matrix" => {
                        let rows = self.rows_until_env_end(false);
                        let delim = match env.as_str() {
                            "bmatrix" => ", delim: \"[\"",
                            "vmatrix" => ", delim: \"|\"",
                            _ => "",
                        };
                        self.last = Last::Atom;
                        format!("mat({rows}{delim})")
                    }
                    "cases" => {
                        let rows = self.rows_until_env_end(true);
                        self.last = Last::Atom;
                        format!("cases({rows})")
                    }
                    // Aligned/multi-line display: typst math keeps `&` and
                    // `\\` natively — pass the body through.
                    _ => {
                        let body = self.until_env_end();
                        self.last = Last::None;
                        body
                    }
                }
            }
            "end" => String::new(), // consumed by the env readers

            // escaped punctuation: literal upright text
            "{" | "}" | "&" | "%" | "$" | "#" | "_" => {
                self.last = Last::Other;
                format!("\"{name}\"")
            }
            "|" => {
                self.last = Last::Other;
                "‖".to_string()
            }

            _ => {
                // Unknown command: degrade to visible upright text — never
                // fail the render, never swallow silently.
                self.last = Last::Other;
                format!("\"{name}\"")
            }
        })
    }

    /// Translates environment body rows for `mat(...)` / `cases(...)`.
    /// Matrices: `&` → cell separator, `\` → row separator. Cases: rows are
    /// branches (`,`), `&` alignment kept inline (verified: typst's `cases`
    /// honours alignment points). Each cell runs through the full sequence
    /// state machine.
    fn rows_until_env_end(&mut self, is_cases: bool) -> String {
        let mut rows: Vec<Vec<Vec<Tok>>> = vec![vec![Vec::new()]];
        while let Some(tok) = self.peek().cloned() {
            match tok {
                Tok::Cmd(c) if c == "end" => {
                    self.next();
                    let _ = self.word_arg(); // the {env} name
                    break;
                }
                Tok::RowBreak => {
                    self.next();
                    rows.push(vec![Vec::new()]);
                }
                Tok::Amp if !is_cases => {
                    self.next();
                    rows.last_mut().unwrap().push(Vec::new());
                }
                _ => {
                    self.next();
                    rows.last_mut().unwrap().last_mut().unwrap().push(tok);
                }
            }
        }
        if is_cases {
            rows.iter()
                .map(|row| {
                    row.iter()
                        .filter_map(|cell| translate_tokens(cell))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            rows.iter()
                .map(|row| {
                    row.iter()
                        .filter_map(|cell| translate_tokens(cell))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .collect::<Vec<_>>()
                .join("; ")
        }
    }

    /// Translates content up to `\end{…}`, keeping `&` and `\` as-is.
    fn until_env_end(&mut self) -> String {
        let mut toks: Vec<Tok> = Vec::new();
        while let Some(tok) = self.peek().cloned() {
            if matches!(&tok, Tok::Cmd(c) if c == "end") {
                self.next();
                let _ = self.word_arg();
                break;
            }
            self.next();
            toks.push(tok);
        }
        translate_tokens(&toks)
            .unwrap_or_default()
            .trim_end()
            .to_string()
    }
}

/// Runs a token slice through a fresh translator (cell-level rendering).
fn translate_tokens(toks: &[Tok]) -> Option<String> {
    if toks.is_empty() {
        return None;
    }
    let mut t = Translator {
        toks,
        pos: 0,
        last: Last::None,
        word_mode: false,
        display: false,
    };
    Some(t.sequence(Stop::End).trim().to_string())
}

/// `\mathbb{X}` → typst blackboard letters (the predefined set); other
/// letters fall back to plain (visible, honest).
fn blackboard(body: &str) -> String {
    let trimmed = body.trim();
    match trimmed {
        "N" => "NN".into(),
        "Z" => "ZZ".into(),
        "Q" => "QQ".into(),
        "R" => "RR".into(),
        "C" => "CC".into(),
        "H" => "HH".into(),
        // Unknown letters: quote — bare multi-letter words would be
        // resolved as typst code variables and fail the compile.
        other => format!("\"{other}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::translate;

    #[test]
    fn corpus_pieces() {
        assert_eq!(
            translate(r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}"),
            "frac(-b ± sqrt(b^(2) - 4 a c), 2 a)"
        );
        assert_eq!(
            translate(r"\sum_{i=1}^{n} i = \frac{n(n+1)}{2}"),
            "sum_(i = 1)^(n) i = frac(n (n + 1), 2)"
        );
        assert_eq!(
            translate(r"\int_{0}^{\infty} e^{-x}\,dx = 1"),
            "integral_(0)^(oo) e^(-x) thin d x = 1"
        );
        assert_eq!(
            translate(r"\lim_{x \to 0} \frac{\sin x}{x}"),
            "lim_(x -> 0) frac(sin x, x)"
        );
        assert_eq!(
            translate(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}"),
            "mat(a, b; c, d)"
        );
        assert_eq!(
            translate(r"\begin{cases} x^2 & x \geq 0 \\ -x & x < 0 \end{cases}"),
            "cases(x^(2) & x >= 0, -x & x < 0)"
        );
        assert_eq!(translate(r"\mathbb{R}^n \mathcal{L}"), "RR^(n) cal(L)");
        assert_eq!(
            translate(r"\hat{y} = \vec{w}^\top"),
            "hat(y) = arrow(w)^(top)"
        );
        assert_eq!(translate(r"\overline{x}"), "overline(x)");
        assert_eq!(translate(r"\text{loss}"), "\"loss\"");
        assert_eq!(translate(r"\sqrt[3]{x}"), "root(3, x)");
        assert_eq!(translate(r"E = mc^2"), "E = m c^(2)");
        assert_eq!(
            translate(r"x \leq y \neq z \approx \infty"),
            "x <= y != z approx oo"
        );
        assert_eq!(
            translate(r"\nabla \cdot \mathbf{E} = \rho / \varepsilon_0"),
            "nabla dot.op bold(E) = rho / epsilon.alt_(0)"
        );
        // Unknown commands degrade to upright text, never to a compile error.
        assert_eq!(translate(r"\nosuchcmd{x}"), "\"nosuchcmd\" x");
    }
}
