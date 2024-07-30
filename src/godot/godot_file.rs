use std::{
    fs::File,
    io::{self, BufReader, Bytes, Read, Write},
    path::Path,
};

use anyhow::{bail, Context, Result};

const FORMAT_VERSION: i64 = 3;

pub fn parse_file<P>(path: P) -> Result<GodotFile>
where
    P: AsRef<Path>,
{
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut tokens = Tokenizer {
        bytes: reader.bytes(),
        saved: None,
    };

    let Some(header) = Tag::parse(&mut tokens).context("could not parse header tag")? else {
        bail!("unexpected empty file");
    };

    if let Some(format_field) = header
        .fields
        .iter()
        .find(|field| field.identifier == "format")
    {
        if !matches!(format_field.value, Value::Integer(FORMAT_VERSION)) {
            bail!("unexpected format version {:?}", format_field.value);
        }
    }

    let mut tags = Vec::new();

    while let Some(mut tag) = Tag::parse(&mut tokens).context("could not parse tag")? {
        while let Some(assign) =
            TagAssign::parse(&mut tokens).context("could not parse tag assign")?
        {
            tag.assigns.push(assign);
        }

        tags.push(tag)
    }

    Ok(GodotFile { header, tags })
}

pub(crate) struct GodotFile {
    pub header: Tag,
    pub tags: Vec<Tag>,
}

pub(crate) struct Tag {
    pub name: String,
    pub fields: Vec<Field>,
    pub assigns: Vec<TagAssign>,
}

impl Tag {
    fn parse(tokens: &mut Tokenizer) -> Result<Option<Self>> {
        match tokens.next_token()? {
            Some(Token::BracketOpen) => {}
            Some(token) => bail!("unexpected token {:?}", token),
            None => return Ok(None),
        };

        let mut name = match tokens.next_token()? {
            Some(Token::Identifier(name)) => name,
            Some(token) => bail!("expected identifier (tag name), but found {token:?}"),
            None => bail!("expected identifier (tag name)"),
        };

        let mut fields = Vec::new();
        let mut parsing_tag = true;

        loop {
            let token = match tokens.next_token()? {
                Some(Token::BracketClose) => break,
                Some(token) => token,
                None => bail!("unexpected end of file while parsing tag '{name}'"),
            };

            if parsing_tag && matches!(token, Token::Period) {
                name += ".";
            } else if parsing_tag && matches!(token, Token::Colon) {
                name += ":";
            } else {
                parsing_tag = false;
            }

            let Token::Identifier(identifier) = token else {
                bail!("expected an identifier, but found {token:?}")
            };

            if parsing_tag {
                name += &identifier;
                continue;
            }

            match tokens.next_token()? {
                Some(Token::Equal) => {}
                Some(token) => bail!("expected '=', but found {token:?}"),
                None => bail!("expected '='"),
            };

            let value = Value::parse(tokens)?;

            fields.push(Field { identifier, value });
        }

        Ok(Some(Tag {
            name,
            fields,
            assigns: Vec::new(),
        }))
    }
}

impl GodotFmt for Tag {
    fn godot_fmt(&self, w: &mut dyn Write) -> io::Result<()> {
        write!(w, "[{}", self.name)?;
        for field in &self.fields {
            write!(w, " {}=", field.identifier)?;
            field.value.godot_fmt(w)?
        }
        writeln!(w, "]")?;

        for assign in &self.assigns {
            write!(w, "{} = ", assign.assign)?;
            assign.value.godot_fmt(w)?;
            writeln!(w, "")?;
        }

        Ok(())
    }
}

pub(crate) struct Field {
    pub identifier: String,
    pub value: Value,
}

pub(crate) struct TagAssign {
    pub assign: String,
    pub value: Value,
}

impl TagAssign {
    fn parse(tokens: &mut Tokenizer) -> Result<Option<Self>> {
        let mut what = String::new();

        loop {
            let Some(character) = tokens.next_byte()? else {
                return Ok(None);
            };

            match character {
                b';' => loop {
                    match tokens.next_byte()? {
                        Some(b'\n') => break,
                        None => return Ok(None),
                        _ => {}
                    }
                },
                b'[' if what.is_empty() => {
                    tokens.save_byte(character);
                    return Ok(None);
                }
                b'"' => {
                    tokens.save_byte(b'"');
                    let Some(Token::String(value)) = tokens.next_token()? else {
                        bail!("expected a quoted string");
                    };

                    what = value;
                }
                b'=' => {
                    return Ok(Some(Self {
                        assign: what,
                        value: Value::parse(tokens)?,
                    }));
                }
                b'\n' => {}
                0..=32 => {}
                _ => what.push(character as char),
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum Value {
    Bool(bool),
    Null,
    Integer(i64),
    Double(f64),
    String(String),
    StringName(String),
    Color(Color),
    Vector2i(Vector2i),
    SubResource(String),
    ExtResource(String),
}

impl Value {
    fn parse(tokens: &mut Tokenizer) -> Result<Self> {
        match tokens.next_token()? {
            Some(Token::Identifier(id)) => match &*id {
                "true" => Ok(Self::Bool(true)),
                "false" => Ok(Self::Bool(false)),
                "null" | "nil" => Ok(Self::Null),
                "inf" => Ok(Self::Double(f64::INFINITY)),
                "neg_inf" => Ok(Self::Double(f64::NEG_INFINITY)),
                "nan" => Ok(Self::Double(f64::NAN)),
                "Vector2i" => {
                    let args = Self::parse_int_constructor(tokens)?;

                    let [x, y] = *args else {
                        bail!("Vector2i requires 2 arguments");
                    };

                    Ok(Self::Vector2i(Vector2i { x, y }))
                }
                "Color" => {
                    let args = Self::parse_double_constructor(tokens)?;

                    let [r, g, b, a] = *args else {
                        bail!("Color requires 4 arguments");
                    };

                    Ok(Self::Color(Color::Rgba(r, g, b, a)))
                }
                "SubResource" => {
                    match tokens.next_token()? {
                        Some(Token::ParenthesisOpen) => {}
                        Some(token) => bail!("expected '(', but found {:?}", token),
                        None => bail!("expected '('"),
                    };

                    let value = match tokens.next_token()? {
                        Some(Token::String(value)) => value,
                        Some(token) => bail!(
                            "expected a string argument to SubResource(), but found {:?}",
                            token
                        ),
                        None => bail!("expected a string argument to SubResource()"),
                    };

                    match tokens.next_token()? {
                        Some(Token::ParenthesisClose) => {}
                        Some(token) => bail!("expected ')', but found {:?}", token),
                        None => bail!("expected ')'"),
                    };

                    Ok(Self::SubResource(value))
                }
                "ExtResource" => {
                    match tokens.next_token()? {
                        Some(Token::ParenthesisOpen) => {}
                        Some(token) => bail!("expected '(', but found {:?}", token),
                        None => bail!("expected '('"),
                    };

                    let value = match tokens.next_token()? {
                        Some(Token::String(value)) => value,
                        Some(token) => bail!(
                            "expected a string argument to ExtResource(), but found {:?}",
                            token
                        ),
                        None => bail!("expected a string argument to ExtResource()"),
                    };

                    match tokens.next_token()? {
                        Some(Token::ParenthesisClose) => {}
                        Some(token) => bail!("expected ')', but found {:?}", token),
                        None => bail!("expected ')'"),
                    };

                    Ok(Self::ExtResource(value))
                }
                _ => bail!("unsupported or unexpected value identifier '{id}'"),
            },
            Some(Token::Integer(value)) => Ok(Self::Integer(value)),
            Some(Token::Double(value)) => Ok(Self::Double(value)),
            Some(Token::String(value)) => Ok(Self::String(value)),
            Some(Token::StringName(value)) => Ok(Self::StringName(value)),
            Some(Token::Color(value)) => Ok(Self::Color(Color::Html(value))),
            Some(token) => bail!("unsupported or unexpected value token {token:?}"),
            None => bail!("expected a value, but found end of file"),
        }
    }

    fn parse_int_constructor(tokens: &mut Tokenizer) -> Result<Vec<i64>> {
        let mut args = Vec::new();

        match tokens.next_token()? {
            Some(Token::ParenthesisOpen) => {}
            Some(token) => bail!("expected '(', but found {:?}", token),
            None => bail!("expected '('"),
        };

        loop {
            if !args.is_empty() {
                match tokens.next_token()? {
                    Some(Token::Comma) => {}
                    Some(Token::ParenthesisClose) => break,
                    Some(token) => bail!("expected ',' or ')', but found {:?}", token),
                    None => bail!("expected ',' or ')'"),
                };
            }

            let value = match tokens.next_token()? {
                Some(Token::Integer(value)) => value,
                Some(Token::ParenthesisClose) if args.is_empty() => break,
                Some(token) => bail!("expected integer, but found {:?}", token),
                None => bail!("expected integer"),
            };

            args.push(value);
        }

        Ok(args)
    }

    fn parse_double_constructor(tokens: &mut Tokenizer) -> Result<Vec<f64>> {
        let mut args = Vec::new();

        match tokens.next_token()? {
            Some(Token::ParenthesisOpen) => {}
            Some(token) => bail!("expected '(', but found {:?}", token),
            None => bail!("expected '('"),
        };

        loop {
            if !args.is_empty() {
                match tokens.next_token()? {
                    Some(Token::Comma) => {}
                    Some(Token::ParenthesisClose) => break,
                    Some(token) => bail!("expected ',' or ')', but found {:?}", token),
                    None => bail!("expected ',' or ')'"),
                };
            }

            let value = match tokens.next_token()? {
                Some(Token::Integer(value)) => value as f64,
                Some(Token::Double(value)) => value,
                Some(Token::ParenthesisClose) if args.is_empty() => break,
                Some(token) => bail!("expected float, but found {:?}", token),
                None => bail!("expected float"),
            };

            args.push(value);
        }

        Ok(args)
    }
}

impl GodotFmt for Value {
    fn godot_fmt(&self, w: &mut dyn Write) -> io::Result<()> {
        match self {
            Value::Null => write!(w, "null"),
            Value::Bool(true) => write!(w, "true"),
            Value::Bool(false) => write!(w, "false"),
            Value::Integer(value) => value.godot_fmt(w),
            Value::Double(value) => {
                let mut string = Vec::new();
                value.godot_fmt(&mut string)?;

                if string != b"inf" && string != b"inf_neg" && string != b"nan" {
                    if !string.contains(&b'.') && !string.contains(&b'e') {
                        string.extend_from_slice(b".0");
                    }
                }

                w.write_all(&string)
            }
            Value::String(value) => write!(w, r#""{value}""#),
            Value::StringName(value) => write!(w, r#"&"{value}""#),
            Value::Color(value) => value.godot_fmt(w),
            Value::Vector2i(value) => value.godot_fmt(w),
            Value::SubResource(value) => write!(w, r#"SubResource("{value}")"#),
            Value::ExtResource(value) => write!(w, r#"ExtResource("{value}")"#),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Vector2i {
    pub x: i64,
    pub y: i64,
}

impl GodotFmt for Vector2i {
    fn godot_fmt(&self, w: &mut dyn Write) -> io::Result<()> {
        write!(w, "Vector2i(")?;
        self.x.godot_fmt(w)?;
        write!(w, ", ")?;
        self.y.godot_fmt(w)?;
        write!(w, ")")
    }
}

impl From<[u32; 2]> for Vector2i {
    fn from([x, y]: [u32; 2]) -> Self {
        Self {
            x: x as i64,
            y: y as i64,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Color {
    Rgba(f64, f64, f64, f64),
    Html(String),
}

impl GodotFmt for Color {
    fn godot_fmt(&self, w: &mut dyn Write) -> io::Result<()> {
        match self {
            Color::Rgba(r, g, b, a) => {
                write!(w, "Color(")?;
                r.godot_fmt(w)?;
                write!(w, ", ")?;
                g.godot_fmt(w)?;
                write!(w, ", ")?;
                b.godot_fmt(w)?;
                write!(w, ", ")?;
                a.godot_fmt(w)?;
                write!(w, ")")
            }
            Color::Html(value) => write!(w, "{value}"),
        }
    }
}

#[derive(Debug)]
enum Token {
    CurlyBracketOpen,
    CurlyBracketClose,
    BracketOpen,
    BracketClose,
    ParenthesisOpen,
    ParenthesisClose,
    Identifier(String),
    String(String),
    StringName(String),
    Integer(i64),
    Double(f64),
    Color(String),
    Colon,
    Comma,
    Period,
    Equal,
}

struct Tokenizer {
    bytes: Bytes<BufReader<File>>,
    saved: Option<u8>,
}

impl Tokenizer {
    fn next_byte(&mut self) -> Result<Option<u8>> {
        if let Some(c) = self.saved.take() {
            Ok(Some(c))
        } else if let Some(c) = self.bytes.next() {
            Ok(Some(c?))
        } else {
            return Ok(None);
        }
    }

    fn save_byte(&mut self, byte: u8) {
        assert!(self.saved.is_none());
        self.saved = Some(byte);
    }

    fn next_token(&mut self) -> Result<Option<Token>> {
        loop {
            let Some(character) = self.next_byte()? else {
                return Ok(None);
            };

            match character {
                b'{' => return Ok(Some(Token::CurlyBracketOpen)),
                b'}' => return Ok(Some(Token::CurlyBracketClose)),
                b'[' => return Ok(Some(Token::BracketOpen)),
                b']' => return Ok(Some(Token::BracketClose)),
                b'(' => return Ok(Some(Token::ParenthesisOpen)),
                b')' => return Ok(Some(Token::ParenthesisClose)),
                b':' => return Ok(Some(Token::Colon)),
                b';' => loop {
                    match self.next_byte()? {
                        Some(b'\n') => break,
                        None => return Ok(None),
                        _ => {}
                    }
                },
                b',' => return Ok(Some(Token::Comma)),
                b'.' => return Ok(Some(Token::Period)),
                b'=' => return Ok(Some(Token::Equal)),
                b'#' => {
                    let mut color_str = String::from("#");

                    loop {
                        match self.next_byte()? {
                            None => return Ok(None),
                            Some(c) if c.is_ascii_hexdigit() => color_str.push(c as char),
                            Some(c) => {
                                self.save_byte(c);
                                break;
                            }
                        }
                    }

                    return Ok(Some(Token::Color(color_str)));
                }
                b'"' | b'@' | b'&' => {
                    // StringName
                    let is_string_name = if matches!(character, b'@' | b'&') {
                        if self.next_byte()? != Some(b'"') {
                            bail!("expected '\"' after '&'");
                        }

                        true
                    } else {
                        false
                    };

                    let mut string = String::new();

                    // Preserves escape sequences. Change it if we want to parse the content.
                    loop {
                        match self.next_byte()? {
                            None => bail!("unterminated string"),
                            Some(b'"') => break,
                            Some(b'\n') => {}
                            Some(c) => string.push(c as char),
                        }
                    }

                    if is_string_name {
                        return Ok(Some(Token::StringName(string)));
                    } else {
                        return Ok(Some(Token::String(string)));
                    }
                }
                b'-' | b'0'..=b'9' => {
                    #[derive(Clone, Copy, PartialEq, Eq)]
                    enum Reading {
                        Int,
                        Dec,
                        Exp,
                        Done,
                    }

                    let mut num = String::new();
                    let mut reading = Reading::Int;

                    let mut next = if character == b'-' {
                        num.push('-');

                        self.next_byte()?
                    } else {
                        Some(character)
                    };

                    let mut is_float = false;
                    let mut exp_begin = false;
                    let mut exp_sign = false;

                    while let Some(current) = next {
                        match reading {
                            Reading::Int => match current {
                                b'0'..=b'9' => {}
                                b'.' => {
                                    reading = Reading::Dec;
                                    is_float = true;
                                }
                                b'e' => {
                                    reading = Reading::Exp;
                                    is_float = true;
                                }
                                _ => reading = Reading::Done,
                            },
                            Reading::Dec => match current {
                                b'0'..=b'9' => {}
                                b'e' => {
                                    reading = Reading::Exp;
                                    is_float = true;
                                }
                                _ => reading = Reading::Done,
                            },
                            Reading::Exp => match current {
                                b'0'..=b'9' => {
                                    exp_begin = true;
                                }
                                b'-' | b'+' if !exp_begin && !exp_sign => {
                                    exp_sign = true;
                                }
                                _ => reading = Reading::Done,
                            },
                            Reading::Done => break,
                        }

                        if reading == Reading::Done {
                            break;
                        }

                        num.push(current as char);
                        next = self.next_byte()?;
                    }

                    self.saved = next;

                    if is_float {
                        return Ok(Some(Token::Double(
                            num.parse()
                                .with_context(|| format!("could not parse {num:?} as double"))?,
                        )));
                    } else {
                        return Ok(Some(Token::Integer(
                            num.parse()
                                .with_context(|| format!("could not parse {num:?} as int"))?,
                        )));
                    }
                }
                character if character.is_ascii_alphabetic() || character == b'_' => {
                    let mut id = String::new();
                    id.push(character as char);

                    while let Some(character) = self.next_byte()? {
                        if character.is_ascii_alphanumeric() || character == b'_' {
                            id.push(character as char);
                        } else {
                            self.save_byte(character);
                            break;
                        }
                    }

                    return Ok(Some(Token::Identifier(id)));
                }
                0..=32 => {}
                _ => bail!("unexpected character '{}'", character as char),
            }
        }
    }
}

trait GodotFmt {
    fn godot_fmt(&self, w: &mut dyn Write) -> io::Result<()>;
}

impl GodotFmt for i64 {
    fn godot_fmt(&self, w: &mut dyn Write) -> io::Result<()> {
        const BASE: i64 = 10;

        let sign = *self < 0;
        let mut n = *self;

        let mut chars = 0;
        loop {
            n /= BASE;
            chars += 1;

            if n == 0 {
                break;
            }
        }

        if sign {
            chars += 1;
        }

        let mut string = vec![b' '; chars];
        n = *self;
        loop {
            let modulus = (n % BASE).abs() as u8;

            chars -= 1;
            if modulus >= 10 {
                let a = b'a';
                string[chars] = a + (modulus - 10);
            } else {
                string[chars] = b'0' + modulus;
            }

            n /= BASE;

            if n == 0 {
                break;
            }
        }

        if sign {
            string[0] = b'-';
        }

        w.write_all(&string)
    }
}

impl GodotFmt for f64 {
    fn godot_fmt(&self, w: &mut dyn Write) -> io::Result<()> {
        // Corresponds to `rtos_fix`.

        if *self == 0.0 {
            w.write_all(b"0")
        } else if self.is_nan() {
            w.write_all(b"nan")
        } else if self.is_infinite() {
            if *self > 0.0 {
                w.write_all(b"inf")
            } else {
                w.write_all(b"neg_inf")
            }
        } else {
            // Godot uses `s[n]printf` and a bunch of compiler specific settings
            // to get the C99 output format. `ryu` uses scientific notation for
            // long numbers, at least, but adds ".0" to integers. Trimming away
            // the ".0" will have to do for now.
            let mut buffer = ryu::Buffer::new();
            w.write_all(
                buffer
                    .format_finite(*self)
                    .trim_end_matches(".0")
                    .as_bytes(),
            )
        }
    }
}

pub(crate) struct GodotWriter<W> {
    writer: W,
}

impl<W: Write> GodotWriter<W> {
    pub(crate) fn begin(mut writer: W, header: &Tag) -> Result<Self> {
        header.godot_fmt(&mut writer)?;
        Ok(Self { writer })
    }

    pub(crate) fn write_tag(&mut self, tag: &Tag) -> Result<()> {
        writeln!(self.writer, "")?;
        tag.godot_fmt(&mut self.writer)?;

        Ok(())
    }
}
