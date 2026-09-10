use cssparser::{Parser, Token};

/// An sRGB color, with unpremultiplied alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

/// A length retained until its containing block is known.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum Length {
    Px(f32),
    Em(f32),
    Rem(f32),
    Percent(f32),
    Auto,
}
impl Length {
    /// Resolve against the containing width, element font, and root font.
    pub fn resolve(self, basis: f32, font: f32, root: f32) -> f32 {
        match self {
            Self::Px(v) => v,
            Self::Em(v) => v * font,
            Self::Rem(v) => v * root,
            Self::Percent(v) => v * basis,
            Self::Auto => 0.0,
        }
        .clamp(-10_000.0, 10_000.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum Display {
    Inline,
    Block,
    None,
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum WhiteSpace {
    Normal,
    Pre,
    PreWrap,
    NoWrap,
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum TextAlign {
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum LineHeight {
    Number(f32),
    Px(f32),
}

/// Computed text styles and the supported block box properties. Lengths in the
/// box model are resolved by the renderer; inherited font sizes are absolute.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub struct ComputedStyle {
    pub color: Color,
    pub background: Color,
    pub font_size: f32,
    pub font_weight: f32,
    pub italic: bool,
    pub monospace: bool,
    pub underline: bool,
    pub strike: bool,
    pub line_height: LineHeight,
    pub white_space: WhiteSpace,
    pub text_align: TextAlign,
    pub display: Display,
    /// Top, right, bottom, left.
    pub margin: [Length; 4],
    /// Top, right, bottom, left.
    pub padding: [Length; 4],
    pub width: Length,
    pub max_width: Length,
    pub height: Length,
    pub border_width: Length,
    pub border_color: Color,
    pub border_solid: bool,
    pub border_radius: Length,
}
impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            color: Color(38, 44, 32, 255),
            background: Color(0, 0, 0, 0),
            font_size: 17.0,
            font_weight: 400.0,
            italic: false,
            monospace: false,
            underline: false,
            strike: false,
            line_height: LineHeight::Number(1.45),
            white_space: WhiteSpace::Normal,
            text_align: TextAlign::Left,
            display: Display::Inline,
            margin: [Length::Px(0.0); 4],
            padding: [Length::Px(0.0); 4],
            width: Length::Auto,
            max_width: Length::Auto,
            height: Length::Auto,
            border_width: Length::Px(3.0),
            border_color: Color(38, 44, 32, 255),
            border_solid: false,
            border_radius: Length::Px(0.0),
        }
    }
}
impl ComputedStyle {
    pub(super) fn inherited(parent: Self) -> Self {
        Self {
            color: parent.color,
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            italic: parent.italic,
            monospace: parent.monospace,
            underline: parent.underline,
            strike: parent.strike,
            line_height: parent.line_height,
            white_space: parent.white_space,
            text_align: parent.text_align,
            border_color: parent.color,
            ..Self::default()
        }
    }
    pub fn line_height_px(self) -> f32 {
        match self.line_height {
            LineHeight::Number(n) => n * self.font_size,
            LineHeight::Px(px) => px,
        }
        .clamp(1.0, 1024.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Property {
    FontSize,
    Color,
    Background,
    FontWeight,
    Italic,
    Family,
    Decoration,
    LineHeight,
    WhiteSpace,
    Align,
    Display,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Width,
    MaxWidth,
    Height,
    BorderWidth,
    BorderColor,
    BorderStyle,
    BorderRadius,
    BoxShadow,
}
pub(super) const PROPERTIES: [Property; 27] = [
    Property::FontSize,
    Property::Color,
    Property::Background,
    Property::FontWeight,
    Property::Italic,
    Property::Family,
    Property::Decoration,
    Property::LineHeight,
    Property::WhiteSpace,
    Property::Align,
    Property::Display,
    Property::MarginTop,
    Property::MarginRight,
    Property::MarginBottom,
    Property::MarginLeft,
    Property::PaddingTop,
    Property::PaddingRight,
    Property::PaddingBottom,
    Property::PaddingLeft,
    Property::Width,
    Property::MaxWidth,
    Property::Height,
    Property::BorderWidth,
    Property::BorderColor,
    Property::BorderStyle,
    Property::BorderRadius,
    Property::BoxShadow,
];
impl Property {
    pub(super) fn inherited(self) -> bool {
        matches!(
            self,
            Self::FontSize
                | Self::Color
                | Self::FontWeight
                | Self::Italic
                | Self::Family
                | Self::LineHeight
                | Self::WhiteSpace
                | Self::Align
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Value {
    Color(Color),
    CurrentColor,
    Length(Length),
    Number(f32),
    Bool(bool),
    Decoration(bool, bool),
    WhiteSpace(WhiteSpace),
    Align(TextAlign),
    Display(Display),
    Inherit,
    Initial,
    Unset,
}
#[derive(Clone, Debug)]
pub(super) struct Declaration {
    pub property: Property,
    pub value: Value,
    pub important: bool,
}
type Error<'i> = cssparser::ParseError<'i, ()>;

fn keyword<'i>(input: &mut Parser<'i, '_>) -> Result<String, Error<'i>> {
    Ok(input.expect_ident()?.to_ascii_lowercase())
}
fn length<'i>(
    input: &mut Parser<'i, '_>,
    auto: bool,
    negative: bool,
    percent: bool,
) -> Result<Length, Error<'i>> {
    let value = match input.next()? {
        Token::Number { value: 0.0, .. } => Length::Px(0.0),
        Token::Dimension { value, unit, .. }
            if value.is_finite() && (negative || *value >= 0.0) =>
        {
            match unit.to_ascii_lowercase().as_str() {
                "px" => Length::Px(*value),
                "em" => Length::Em(*value),
                "rem" => Length::Rem(*value),
                _ => return Err(input.new_custom_error(())),
            }
        }
        Token::Percentage { unit_value, .. }
            if percent && unit_value.is_finite() && (negative || *unit_value >= 0.0) =>
        {
            Length::Percent(*unit_value)
        }
        Token::Ident(s) if auto && s.eq_ignore_ascii_case("auto") => Length::Auto,
        _ => return Err(input.new_custom_error(())),
    };
    Ok(value)
}
fn component<'i>(input: &mut Parser<'i, '_>, alpha: bool) -> Result<f32, Error<'i>> {
    let n = match input.next()? {
        Token::Number { value, .. } => {
            if alpha {
                *value
            } else {
                *value / 255.0
            }
        }
        Token::Percentage { unit_value, .. } => *unit_value,
        _ => return Err(input.new_custom_error(())),
    };
    if n.is_finite() {
        Ok(n.clamp(0.0, 1.0))
    } else {
        Err(input.new_custom_error(()))
    }
}
fn color<'i>(input: &mut Parser<'i, '_>) -> Result<Value, Error<'i>> {
    let token = input.next()?.clone();
    let rgba = match token {
        Token::Hash(v) | Token::IDHash(v) => {
            let (r, g, b, a) = cssparser::color::parse_hash_color(v.as_bytes())
                .map_err(|_| input.new_custom_error(()))?;
            Color(r, g, b, (a * 255.0).round() as u8)
        }
        Token::Ident(v) if v.eq_ignore_ascii_case("currentcolor") => {
            return Ok(Value::CurrentColor);
        }
        Token::Ident(v) if v.eq_ignore_ascii_case("transparent") => Color(0, 0, 0, 0),
        Token::Ident(v) => {
            let (r, g, b) =
                cssparser::color::parse_named_color(&v).map_err(|_| input.new_custom_error(()))?;
            Color(r, g, b, 255)
        }
        Token::Function(v) if v.eq_ignore_ascii_case("rgb") || v.eq_ignore_ascii_case("rgba") => {
            input.parse_nested_block(|input| {
                let r = component(input, false)?;
                let commas = input.try_parse(|p| p.expect_comma()).is_ok();
                let g = component(input, false)?;
                if commas {
                    input.expect_comma()?;
                }
                let b = component(input, false)?;
                let alpha = if input
                    .try_parse(|p| {
                        if commas {
                            p.expect_comma()
                        } else {
                            p.expect_delim('/')
                        }
                    })
                    .is_ok()
                {
                    component(input, true)?
                } else {
                    1.0
                };
                input.expect_exhausted()?;
                Ok(Color(
                    (r * 255.0).round() as u8,
                    (g * 255.0).round() as u8,
                    (b * 255.0).round() as u8,
                    (alpha * 255.0).round() as u8,
                ))
            })?
        }
        _ => return Err(input.new_custom_error(())),
    };
    Ok(Value::Color(rgba))
}

pub(super) fn parse_values<'i>(
    name: &str,
    input: &mut Parser<'i, '_>,
) -> Result<Vec<(Property, Value)>, Error<'i>> {
    use Property as P;
    let props: &[P] = match name {
        "color" => &[P::Color],
        "background" | "background-color" => &[P::Background],
        "font-size" => &[P::FontSize],
        "font-weight" => &[P::FontWeight],
        "font-style" => &[P::Italic],
        "font-family" => &[P::Family],
        "text-decoration" | "text-decoration-line" => &[P::Decoration],
        "line-height" => &[P::LineHeight],
        "white-space" => &[P::WhiteSpace],
        "text-align" => &[P::Align],
        "display" => &[P::Display],
        "margin" => &[P::MarginTop, P::MarginRight, P::MarginBottom, P::MarginLeft],
        "margin-top" => &[P::MarginTop],
        "margin-right" => &[P::MarginRight],
        "margin-bottom" => &[P::MarginBottom],
        "margin-left" => &[P::MarginLeft],
        "padding" => &[
            P::PaddingTop,
            P::PaddingRight,
            P::PaddingBottom,
            P::PaddingLeft,
        ],
        "padding-top" => &[P::PaddingTop],
        "padding-right" => &[P::PaddingRight],
        "padding-bottom" => &[P::PaddingBottom],
        "padding-left" => &[P::PaddingLeft],
        "width" => &[P::Width],
        "max-width" => &[P::MaxWidth],
        "height" => &[P::Height],
        "border" => &[P::BorderWidth, P::BorderStyle, P::BorderColor],
        "border-width" => &[P::BorderWidth],
        "border-style" => &[P::BorderStyle],
        "border-color" => &[P::BorderColor],
        "border-radius" => &[P::BorderRadius],
        "box-shadow" => &[P::BoxShadow],
        _ => return Err(input.new_custom_error(())),
    };
    if let Ok(wide) = input.try_parse(|p| -> Result<Value, Error<'i>> {
        match keyword(p)?.as_str() {
            "inherit" => Ok(Value::Inherit),
            "initial" => Ok(Value::Initial),
            "unset" => Ok(Value::Unset),
            _ => Err(p.new_custom_error(())),
        }
    }) {
        return Ok(props.iter().map(|p| (*p, wide)).collect());
    }
    if name == "margin" || name == "padding" {
        let margin = name == "margin";
        let mut values = vec![length(input, margin, margin, true)?];
        while values.len() < 4 {
            if let Ok(v) = input.try_parse(|p| length(p, margin, margin, true)) {
                values.push(v);
            } else {
                break;
            }
        }
        let edges = match values.as_slice() {
            [a] => [*a; 4],
            [a, b] => [*a, *b, *a, *b],
            [a, b, c] => [*a, *b, *c, *b],
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => unreachable!(),
        };
        return Ok(props
            .iter()
            .zip(edges)
            .map(|(p, v)| (*p, Value::Length(v)))
            .collect());
    }
    if name == "border" {
        let mut width = None;
        let mut solid = None;
        let mut ink = None;
        for _ in 0..3 {
            if width.is_none() {
                if let Ok(v) = input.try_parse(border_width) {
                    width = Some(v);
                    continue;
                }
            }
            if solid.is_none() {
                if let Ok(v) = input.try_parse(border_style) {
                    solid = Some(v);
                    continue;
                }
            }
            if ink.is_none() {
                if let Ok(v) = input.try_parse(color) {
                    ink = Some(v);
                    continue;
                }
            }
            break;
        }
        if width.is_none() && solid.is_none() && ink.is_none() {
            return Err(input.new_custom_error(()));
        }
        return Ok(vec![
            (
                P::BorderWidth,
                Value::Length(width.unwrap_or(Length::Px(3.0))),
            ),
            (P::BorderStyle, Value::Bool(solid.unwrap_or(false))),
            (P::BorderColor, ink.unwrap_or(Value::CurrentColor)),
        ]);
    }
    let p = props[0];
    let value = match p {
        P::Color | P::Background | P::BorderColor => color(input)?,
        P::FontSize => {
            let size = input
                .try_parse(|p| length(p, false, false, true))
                .or_else(|_| {
                    Ok(match keyword(input)?.as_str() {
                        "xx-small" => Length::Px(10.0),
                        "x-small" => Length::Px(12.0),
                        "small" => Length::Px(14.0),
                        "medium" => Length::Px(17.0),
                        "large" => Length::Px(20.0),
                        "x-large" => Length::Px(26.0),
                        "xx-large" => Length::Px(34.0),
                        "smaller" => Length::Em(0.8),
                        "larger" => Length::Em(1.2),
                        _ => return Err(input.new_custom_error(())),
                    })
                })?;
            Value::Length(size)
        }
        P::FontWeight => {
            let weight = input.try_parse(|p| p.expect_number()).or_else(|_| {
                match keyword(input)?.as_str() {
                    "normal" => Ok(400.0),
                    "bold" => Ok(700.0),
                    _ => Err(input.new_custom_error(())),
                }
            })?;
            if !weight.is_finite() || !(1.0..=1000.0).contains(&weight) {
                return Err(input.new_custom_error(()));
            }
            Value::Number(weight)
        }
        P::Italic => Value::Bool(match keyword(input)?.as_str() {
            "normal" => false,
            "italic" | "oblique" => true,
            _ => return Err(input.new_custom_error(())),
        }),
        P::Family => {
            let names = input.parse_comma_separated(|p| {
                let mut words = vec![];
                while let Ok(word) = p.try_parse(|p| p.expect_ident_or_string().cloned()) {
                    words.push(word.to_ascii_lowercase());
                }
                if words.is_empty() {
                    return Err(p.new_custom_error(()));
                }
                Ok(words.join(" "))
            })?;
            let mono = names
                .iter()
                .find_map(|s| match s.as_str() {
                    "monospace" | "ui-monospace" | "courier" | "courier new" => Some(true),
                    "serif" | "sans-serif" | "system-ui" | "inter" | "arial" => Some(false),
                    _ => None,
                })
                .unwrap_or(false);
            Value::Bool(mono)
        }
        P::Decoration => {
            if input.try_parse(|p| p.expect_ident_matching("none")).is_ok() {
                Value::Decoration(false, false)
            } else {
                let mut underline = false;
                let mut strike = false;
                for _ in 0..2 {
                    let Ok(word) = input.try_parse(keyword) else {
                        break;
                    };
                    match word.as_str() {
                        "underline" if !underline => underline = true,
                        "line-through" if !strike => strike = true,
                        _ => return Err(input.new_custom_error(())),
                    }
                }
                if !underline && !strike {
                    return Err(input.new_custom_error(()));
                }
                Value::Decoration(underline, strike)
            }
        }
        P::LineHeight => {
            if input
                .try_parse(|p| p.expect_ident_matching("normal"))
                .is_ok()
            {
                Value::Number(1.45)
            } else if let Ok(n) = input.try_parse(|p| p.expect_number()) {
                if !n.is_finite() || n < 0.0 {
                    return Err(input.new_custom_error(()));
                }
                Value::Number(n)
            } else {
                Value::Length(length(input, false, false, true)?)
            }
        }
        P::WhiteSpace => Value::WhiteSpace(match keyword(input)?.as_str() {
            "normal" => WhiteSpace::Normal,
            "pre" => WhiteSpace::Pre,
            "pre-wrap" => WhiteSpace::PreWrap,
            "nowrap" => WhiteSpace::NoWrap,
            _ => return Err(input.new_custom_error(())),
        }),
        P::Align => Value::Align(match keyword(input)?.as_str() {
            "left" | "start" => TextAlign::Left,
            "right" | "end" => TextAlign::Right,
            "center" => TextAlign::Center,
            _ => return Err(input.new_custom_error(())),
        }),
        P::Display => Value::Display(match keyword(input)?.as_str() {
            "inline" => Display::Inline,
            "block" | "list-item" | "inline-block" => Display::Block,
            "none" => Display::None,
            _ => return Err(input.new_custom_error(())),
        }),
        P::BorderWidth => Value::Length(border_width(input)?),
        P::BorderStyle => Value::Bool(border_style(input)?),
        P::BorderRadius => Value::Length(length(input, false, false, false)?),
        P::BoxShadow => {
            while input.next().is_ok() {}
            Value::Bool(true)
        }
        P::Width | P::MaxWidth | P::Height => {
            if p == P::MaxWidth && input.try_parse(|p| p.expect_ident_matching("none")).is_ok() {
                Value::Length(Length::Auto)
            } else {
                Value::Length(length(input, true, false, p != P::Height)?)
            }
        }
        P::MarginTop | P::MarginRight | P::MarginBottom | P::MarginLeft => {
            Value::Length(length(input, true, true, true)?)
        }
        P::PaddingTop | P::PaddingRight | P::PaddingBottom | P::PaddingLeft => {
            Value::Length(length(input, false, false, true)?)
        }
    };
    Ok(vec![(p, value)])
}
fn border_width<'i>(input: &mut Parser<'i, '_>) -> Result<Length, Error<'i>> {
    input
        .try_parse(|p| length(p, false, false, false))
        .or_else(|_| {
            Ok(Length::Px(match keyword(input)?.as_str() {
                "thin" => 1.0,
                "medium" => 3.0,
                "thick" => 5.0,
                _ => return Err(input.new_custom_error(())),
            }))
        })
}
fn border_style<'i>(input: &mut Parser<'i, '_>) -> Result<bool, Error<'i>> {
    match keyword(input)?.as_str() {
        "solid" => Ok(true),
        "none" | "hidden" => Ok(false),
        _ => Err(input.new_custom_error(())),
    }
}

pub(super) fn apply(
    style: &mut ComputedStyle,
    parent: ComputedStyle,
    root: f32,
    prop: Property,
    value: Value,
) {
    use Property as P;
    if matches!(value, Value::Inherit | Value::Initial | Value::Unset) {
        let source = if value == Value::Inherit || (value == Value::Unset && prop.inherited()) {
            parent
        } else {
            ComputedStyle::default()
        };
        match prop {
            P::FontSize => style.font_size = source.font_size,
            P::Color => style.color = source.color,
            P::Background => style.background = source.background,
            P::FontWeight => style.font_weight = source.font_weight,
            P::Italic => style.italic = source.italic,
            P::Family => style.monospace = source.monospace,
            P::Decoration => {
                style.underline = source.underline;
                style.strike = source.strike;
            }
            P::LineHeight => style.line_height = source.line_height,
            P::WhiteSpace => style.white_space = source.white_space,
            P::Align => style.text_align = source.text_align,
            P::Display => style.display = source.display,
            P::MarginTop | P::MarginRight | P::MarginBottom | P::MarginLeft => {
                let i = prop as usize - P::MarginTop as usize;
                style.margin[i] = source.margin[i];
            }
            P::PaddingTop | P::PaddingRight | P::PaddingBottom | P::PaddingLeft => {
                let i = prop as usize - P::PaddingTop as usize;
                style.padding[i] = source.padding[i];
            }
            P::Width => style.width = source.width,
            P::MaxWidth => style.max_width = source.max_width,
            P::Height => style.height = source.height,
            P::BorderWidth => style.border_width = source.border_width,
            P::BorderColor => style.border_color = source.border_color,
            P::BorderStyle => style.border_solid = source.border_solid,
            P::BorderRadius => style.border_radius = source.border_radius,
            P::BoxShadow => {}
        }
        return;
    }
    match (prop, value) {
        (P::FontSize, Value::Length(v)) => {
            style.font_size = v
                .resolve(parent.font_size, parent.font_size, root)
                .clamp(1.0, 256.0)
        }
        (P::Color, Value::Color(v)) => style.color = v,
        (P::Color, Value::CurrentColor) => style.color = parent.color,
        (P::Background, Value::Color(v)) => style.background = v,
        (P::Background, Value::CurrentColor) => style.background = style.color,
        (P::BorderColor, Value::Color(v)) => style.border_color = v,
        (P::BorderColor, Value::CurrentColor) => style.border_color = style.color,
        (P::FontWeight, Value::Number(v)) => style.font_weight = v,
        (P::Italic, Value::Bool(v)) => style.italic = v,
        (P::Family, Value::Bool(v)) => style.monospace = v,
        (P::Decoration, Value::Decoration(u, s)) => {
            style.underline = u;
            style.strike = s;
        }
        (P::LineHeight, Value::Number(v)) => style.line_height = LineHeight::Number(v),
        (P::LineHeight, Value::Length(v)) => {
            style.line_height = LineHeight::Px(v.resolve(style.font_size, style.font_size, root))
        }
        (P::WhiteSpace, Value::WhiteSpace(v)) => style.white_space = v,
        (P::Align, Value::Align(v)) => style.text_align = v,
        (P::Display, Value::Display(v)) => style.display = v,
        (P::MarginTop | P::MarginRight | P::MarginBottom | P::MarginLeft, Value::Length(v)) => {
            style.margin[prop as usize - P::MarginTop as usize] = v
        }
        (P::PaddingTop | P::PaddingRight | P::PaddingBottom | P::PaddingLeft, Value::Length(v)) => {
            style.padding[prop as usize - P::PaddingTop as usize] = v
        }
        (P::Width, Value::Length(v)) => style.width = v,
        (P::MaxWidth, Value::Length(v)) => style.max_width = v,
        (P::Height, Value::Length(v)) => style.height = v,
        (P::BorderWidth, Value::Length(v)) => style.border_width = v,
        (P::BorderRadius, Value::Length(v)) => style.border_radius = v,
        (P::BorderStyle, Value::Bool(v)) => style.border_solid = v,
        (P::BoxShadow, Value::Bool(_)) => {}
        _ => {}
    }
}
