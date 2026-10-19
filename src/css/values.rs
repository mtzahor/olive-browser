use cssparser::{Parser, Token};

/// An sRGB color, with unpremultiplied alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxShadow {
    pub offset_x: Length,
    pub offset_y: Length,
    pub blur: Length,
    pub spread: Length,
    pub color: Color,
    pub current_color: bool,
}

/// A length retained until its containing block is known.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum Length {
    Px(f32),
    Em(f32),
    Rem(f32),
    Percent(f32),
    Vw(f32),
    Vh(f32),
    Calc {
        px: f32,
        em: f32,
        rem: f32,
        percent: f32,
        vw: f32,
        vh: f32,
    },
    Auto,
}
impl Length {
    /// Resolve against the containing width, element font, and root font.
    pub fn resolve(self, basis: f32, font: f32, root: f32) -> f32 {
        self.resolve_with_viewport(basis, font, root, 1024.0, 768.0)
    }
    pub fn resolve_with_viewport(
        self,
        basis: f32,
        font: f32,
        root: f32,
        viewport_width: f32,
        viewport_height: f32,
    ) -> f32 {
        match self {
            Self::Px(v) => v,
            Self::Em(v) => v * font,
            Self::Rem(v) => v * root,
            Self::Percent(v) => v * basis,
            Self::Vw(v) => v * viewport_width / 100.0,
            Self::Vh(v) => v * viewport_height / 100.0,
            Self::Calc {
                px,
                em,
                rem,
                percent,
                vw,
                vh,
            } => {
                px + em * font
                    + rem * root
                    + percent * basis
                    + vw * viewport_width / 100.0
                    + vh * viewport_height / 100.0
            }
            Self::Auto => 0.0,
        }
        .clamp(-10_000.0, 10_000.0)
    }
    fn with_viewport(self, viewport_width: f32, viewport_height: f32) -> Self {
        match self {
            Self::Vw(value) => Self::Px(value * viewport_width / 100.0),
            Self::Vh(value) => Self::Px(value * viewport_height / 100.0),
            Self::Calc {
                px,
                em,
                rem,
                percent,
                vw,
                vh,
            } => Self::Calc {
                px: px + vw * viewport_width / 100.0 + vh * viewport_height / 100.0,
                em,
                rem,
                percent,
                vw: 0.0,
                vh: 0.0,
            },
            value => value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum Display {
    Inline,
    Block,
    Flex,
    Grid,
    None,
    InlineBlock,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum FloatSide {
    #[default]
    None,
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum JustifyContent {
    #[default]
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
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
    Start,
    End,
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
    Auto,
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
    pub direction: Direction,
    pub display: Display,
    pub float_side: FloatSide,
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Length,
    pub row_gap: Length,
    pub column_gap: Length,
    pub position: Position,
    /// Top, right, bottom, left offsets. `auto` means no offset.
    pub inset: [Length; 4],
    /// Equal fractional grid tracks; clamped to twelve columns.
    pub grid_columns: u8,
    pub opacity: f32,
    pub list_marker: bool,
    /// Top, right, bottom, left.
    pub margin: [Length; 4],
    /// Top, right, bottom, left.
    pub padding: [Length; 4],
    pub width: Length,
    pub min_width: Length,
    pub max_width: Length,
    pub height: Length,
    pub min_height: Length,
    pub border_box: bool,
    pub border_width: Length,
    pub border_color: Color,
    pub border_solid: bool,
    pub border_radius: Length,
    pub box_shadow: Option<BoxShadow>,
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
            text_align: TextAlign::Start,
            direction: Direction::Ltr,
            display: Display::Inline,
            float_side: FloatSide::None,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::Start,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Length::Auto,
            row_gap: Length::Px(0.0),
            column_gap: Length::Px(0.0),
            position: Position::Static,
            inset: [Length::Auto; 4],
            grid_columns: 1,
            opacity: 1.0,
            list_marker: true,
            margin: [Length::Px(0.0); 4],
            padding: [Length::Px(0.0); 4],
            width: Length::Auto,
            min_width: Length::Px(0.0),
            max_width: Length::Auto,
            height: Length::Auto,
            min_height: Length::Px(0.0),
            border_box: false,
            border_width: Length::Px(3.0),
            border_color: Color(38, 44, 32, 255),
            border_solid: false,
            border_radius: Length::Px(0.0),
            box_shadow: None,
        }
    }
}
impl ComputedStyle {
    pub(super) fn inherited(parent: Self) -> Self {
        Self {
            color: parent.color,
            opacity: parent.opacity,
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            italic: parent.italic,
            monospace: parent.monospace,
            underline: parent.underline,
            strike: parent.strike,
            line_height: parent.line_height,
            white_space: parent.white_space,
            text_align: parent.text_align,
            direction: parent.direction,
            list_marker: parent.list_marker,
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
    Direction,
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
    MinHeight,
    BoxSizing,
    BorderWidth,
    BorderColor,
    BorderStyle,
    BorderRadius,
    BoxShadow,
    FlexDirection,
    FlexWrap,
    JustifyContent,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    RowGap,
    ColumnGap,
    Position,
    Top,
    Right,
    Bottom,
    Left,
    GridColumns,
    Opacity,
    ListMarker,
    Float,
    MinWidth,
}
pub(super) const PROPERTIES: [Property; 48] = [
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
    Property::Direction,
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
    Property::MinHeight,
    Property::BoxSizing,
    Property::BorderWidth,
    Property::BorderColor,
    Property::BorderStyle,
    Property::BorderRadius,
    Property::BoxShadow,
    Property::FlexDirection,
    Property::FlexWrap,
    Property::JustifyContent,
    Property::FlexGrow,
    Property::FlexShrink,
    Property::FlexBasis,
    Property::RowGap,
    Property::ColumnGap,
    Property::Position,
    Property::Top,
    Property::Right,
    Property::Bottom,
    Property::Left,
    Property::GridColumns,
    Property::Opacity,
    Property::ListMarker,
    Property::Float,
    Property::MinWidth,
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
                | Self::Direction
                | Self::ListMarker
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
    Direction(Direction),
    Display(Display),
    Float(FloatSide),
    FlexDirection(FlexDirection),
    FlexWrap(FlexWrap),
    JustifyContent(JustifyContent),
    Position(Position),
    GridColumns(u8),
    BoxShadow(Option<BoxShadow>),
    ListMarker(bool),
    Inherit,
    Initial,
    Unset,
}
#[derive(Clone, Debug)]
pub(super) struct Declaration {
    pub name: String,
    pub raw: String,
    pub values: Vec<(Property, Value)>,
    pub important: bool,
}
type Error = cssparser::ParseError<()>;

fn keyword(input: &mut Parser<'_>) -> Result<String, Error> {
    Ok(input.expect_ident()?.to_ascii_lowercase())
}
fn length(
    input: &mut Parser<'_>,
    auto: bool,
    negative: bool,
    percent: bool,
) -> Result<Length, Error> {
    let value = match input.next()? {
        Token::Number { value: 0.0, .. } => Length::Px(0.0),
        Token::Dimension { value, unit, .. }
            if value.is_finite() && (negative || *value >= 0.0) =>
        {
            match unit.to_ascii_lowercase().as_str() {
                "px" => Length::Px(*value),
                "em" => Length::Em(*value),
                "rem" => Length::Rem(*value),
                "vw" => Length::Vw(*value),
                "vh" => Length::Vh(*value),
                _ => return Err(cssparser::ParseError::custom(())),
            }
        }
        Token::Function(function) if function.eq_ignore_ascii_case("calc") => {
            return input.parse_nested_block(|input| calc_length(input, percent));
        }
        Token::Percentage { unit_value, .. }
            if percent && unit_value.is_finite() && (negative || *unit_value >= 0.0) =>
        {
            Length::Percent(*unit_value)
        }
        Token::Ident(s) if auto && s.eq_ignore_ascii_case("auto") => Length::Auto,
        _ => return Err(cssparser::ParseError::custom(())),
    };
    Ok(value)
}

fn calc_length(input: &mut Parser<'_>, percent: bool) -> Result<Length, Error> {
    let mut px = 0.0;
    let mut em = 0.0;
    let mut rem = 0.0;
    let mut percent_value = 0.0;
    let mut vw = 0.0;
    let mut vh = 0.0;
    let mut sign = 1.0;
    let mut terms = 0;
    while let Ok(token) = input.next() {
        match token {
            Token::Delim('+') => sign = 1.0,
            Token::Delim('-') => sign = -1.0,
            Token::Number { value: 0.0, .. } => {
                px += 0.0;
                terms += 1;
                sign = 1.0;
            }
            Token::Dimension { value, unit, .. } if value.is_finite() => {
                match unit.to_ascii_lowercase().as_str() {
                    "px" => px += sign * value,
                    "em" => em += sign * value,
                    "rem" => rem += sign * value,
                    "vw" => vw += sign * value,
                    "vh" => vh += sign * value,
                    _ => return Err(cssparser::ParseError::custom(())),
                }
                terms += 1;
                sign = 1.0;
            }
            Token::Percentage { unit_value, .. } if percent && unit_value.is_finite() => {
                percent_value += sign * unit_value;
                terms += 1;
                sign = 1.0;
            }
            _ => return Err(cssparser::ParseError::custom(())),
        }
    }
    if terms == 0 {
        return Err(cssparser::ParseError::custom(()));
    }
    Ok(Length::Calc {
        px,
        em,
        rem,
        percent: percent_value,
        vw,
        vh,
    })
}

fn grid_column_count(input: &mut Parser<'_>) -> Result<u8, Error> {
    if let Ok(count) = input.try_parse(|input| {
        input.expect_function_matching("repeat")?;
        input.parse_nested_block(|input| -> Result<u8, Error> {
            let count = input.expect_number()?;
            if !count.is_finite() || count < 1.0 || count > 12.0 || count.fract() != 0.0 {
                return Err(cssparser::ParseError::custom(()));
            }
            input.expect_comma()?;
            match input.next()? {
                Token::Dimension { value, unit, .. }
                    if *value == 1.0 && unit.eq_ignore_ascii_case("fr") => {}
                _ => return Err(cssparser::ParseError::custom(())),
            }
            input.expect_exhausted()?;
            Ok(count as u8)
        })
    }) {
        return Ok(count);
    }

    let mut count = 0u8;
    while let Ok(token) = input.next() {
        match token {
            Token::Dimension { value, unit, .. }
                if *value == 1.0 && unit.eq_ignore_ascii_case("fr") => {}
            _ => return Err(cssparser::ParseError::custom(())),
        }
        count = count.saturating_add(1);
        if count > 12 {
            return Err(cssparser::ParseError::custom(()));
        }
    }
    if count == 0 {
        Err(cssparser::ParseError::custom(()))
    } else {
        Ok(count)
    }
}
fn component(input: &mut Parser<'_>, alpha: bool) -> Result<f32, Error> {
    let n = match input.next()? {
        Token::Number { value, .. } => {
            if alpha {
                *value
            } else {
                *value / 255.0
            }
        }
        Token::Percentage { unit_value, .. } => *unit_value,
        _ => return Err(cssparser::ParseError::custom(())),
    };
    if n.is_finite() {
        Ok(n.clamp(0.0, 1.0))
    } else {
        Err(cssparser::ParseError::custom(()))
    }
}
fn hue(input: &mut Parser<'_>) -> Result<f32, Error> {
    let degrees = match input.next()? {
        Token::Number { value, .. } => *value,
        Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
            "deg" => *value,
            "grad" => *value * 0.9,
            "rad" => value.to_degrees(),
            "turn" => *value * 360.0,
            _ => return Err(cssparser::ParseError::custom(())),
        },
        _ => return Err(cssparser::ParseError::custom(())),
    };
    if degrees.is_finite() {
        Ok(degrees.rem_euclid(360.0))
    } else {
        Err(cssparser::ParseError::custom(()))
    }
}
fn percentage(input: &mut Parser<'_>) -> Result<f32, Error> {
    match input.next()? {
        Token::Percentage { unit_value, .. } if unit_value.is_finite() => {
            Ok(unit_value.clamp(0.0, 1.0))
        }
        _ => Err(cssparser::ParseError::custom(())),
    }
}
fn hsl_color(input: &mut Parser<'_>) -> Result<Color, Error> {
    input.parse_nested_block(|input| {
        let h = hue(input)?;
        let commas = input.try_parse(|p| p.expect_comma()).is_ok();
        let s = percentage(input)?;
        if commas {
            input.expect_comma()?;
        }
        let l = percentage(input)?;
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

        let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = chroma * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 / 60 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let m = l - chroma / 2.0;
        Ok(Color(
            ((r + m) * 255.0).round() as u8,
            ((g + m) * 255.0).round() as u8,
            ((b + m) * 255.0).round() as u8,
            (alpha * 255.0).round() as u8,
        ))
    })
}
fn color(input: &mut Parser<'_>) -> Result<Value, Error> {
    let token = input.next()?.clone();
    let rgba = match token {
        Token::Hash(v) | Token::IDHash(v) => {
            let (r, g, b, a) = cssparser::color::parse_hash_color(v.as_bytes())
                .map_err(|_| cssparser::ParseError::custom(()))?;
            Color(r, g, b, (a * 255.0).round() as u8)
        }
        Token::Ident(v) if v.eq_ignore_ascii_case("currentcolor") => {
            return Ok(Value::CurrentColor);
        }
        Token::Ident(v) if v.eq_ignore_ascii_case("transparent") => Color(0, 0, 0, 0),
        Token::Ident(v) => {
            let (r, g, b) = cssparser::color::parse_named_color(&v)
                .map_err(|_| cssparser::ParseError::custom(()))?;
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
        Token::Function(v) if v.eq_ignore_ascii_case("hsl") || v.eq_ignore_ascii_case("hsla") => {
            hsl_color(input)?
        }
        _ => return Err(cssparser::ParseError::custom(())),
    };
    Ok(Value::Color(rgba))
}

pub(super) fn parse_values(
    name: &str,
    input: &mut Parser<'_>,
) -> Result<Vec<(Property, Value)>, Error> {
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
        "direction" => &[P::Direction],
        "display" => &[P::Display],
        "float" => &[P::Float],
        "position" => &[P::Position],
        "top" => &[P::Top],
        "right" => &[P::Right],
        "bottom" => &[P::Bottom],
        "left" => &[P::Left],
        "grid-template-columns" => &[P::GridColumns],
        "opacity" => &[P::Opacity],
        "flex-direction" => &[P::FlexDirection],
        "flex-wrap" => &[P::FlexWrap],
        "justify-content" => &[P::JustifyContent],
        "flex-grow" => &[P::FlexGrow],
        "flex-shrink" => &[P::FlexShrink],
        "flex-basis" => &[P::FlexBasis],
        "flex" => &[P::FlexGrow, P::FlexShrink, P::FlexBasis],
        "gap" => &[P::RowGap, P::ColumnGap],
        "row-gap" => &[P::RowGap],
        "column-gap" => &[P::ColumnGap],
        "list-style" | "list-style-type" => &[P::ListMarker],
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
        "min-width" => &[P::MinWidth],
        "max-width" => &[P::MaxWidth],
        "height" => &[P::Height],
        "min-height" => &[P::MinHeight],
        "box-sizing" => &[P::BoxSizing],
        "border" => &[P::BorderWidth, P::BorderStyle, P::BorderColor],
        "border-width" => &[P::BorderWidth],
        "border-style" => &[P::BorderStyle],
        "border-color" => &[P::BorderColor],
        "border-radius" => &[P::BorderRadius],
        "box-shadow" => &[P::BoxShadow],
        _ => return Err(cssparser::ParseError::custom(())),
    };
    if let Ok(wide) = input.try_parse(|p| -> Result<Value, Error> {
        match keyword(p)?.as_str() {
            "inherit" => Ok(Value::Inherit),
            "initial" => Ok(Value::Initial),
            "unset" => Ok(Value::Unset),
            _ => Err(cssparser::ParseError::custom(())),
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
    if name == "gap" {
        let row = length(input, false, false, true)?;
        let column = input
            .try_parse(|input| length(input, false, false, true))
            .unwrap_or(row);
        return Ok(vec![
            (P::RowGap, Value::Length(row)),
            (P::ColumnGap, Value::Length(column)),
        ]);
    }
    if name == "flex" {
        let flex_keyword = input.try_parse(keyword).ok();
        let (grow, shrink, basis) = match flex_keyword.as_deref() {
            Some("none") => (0.0, 0.0, Length::Auto),
            Some("auto") => (1.0, 1.0, Length::Auto),
            Some(_) => return Err(cssparser::ParseError::custom(())),
            _ => {
                let first = input.try_parse(|input| input.expect_number());
                if let Ok(grow) = first {
                    if !grow.is_finite() || grow < 0.0 {
                        return Err(cssparser::ParseError::custom(()));
                    }
                    let second = input.try_parse(|input| input.expect_number()).ok();
                    if second.is_some_and(|value| !value.is_finite() || value < 0.0) {
                        return Err(cssparser::ParseError::custom(()));
                    }
                    let basis = input
                        .try_parse(|input| length(input, true, false, true))
                        .unwrap_or(Length::Percent(0.0));
                    (grow, second.unwrap_or(1.0), basis)
                } else {
                    (1.0, 1.0, length(input, true, false, true)?)
                }
            }
        };
        return Ok(vec![
            (P::FlexGrow, Value::Number(grow.min(1_000_000.0))),
            (P::FlexShrink, Value::Number(shrink.min(1_000_000.0))),
            (P::FlexBasis, Value::Length(basis)),
        ]);
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
            return Err(cssparser::ParseError::custom(()));
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
    if name == "box-shadow" {
        if input
            .try_parse(|input| input.expect_ident_matching("none"))
            .is_ok()
        {
            return Ok(vec![(P::BoxShadow, Value::BoxShadow(None))]);
        }
        let mut lengths = Vec::new();
        let mut shadow_color = None;
        let mut current_color = true;
        let mut color_seen = false;
        while !input.is_exhausted() {
            if let Ok(value) = input.try_parse(|input| length(input, false, true, false)) {
                if lengths.len() == 4 {
                    return Err(cssparser::ParseError::custom(()));
                }
                lengths.push(value);
            } else if !color_seen {
                match input.try_parse(color) {
                    Ok(Value::Color(value)) => {
                        shadow_color = Some(value);
                        current_color = false;
                        color_seen = true;
                    }
                    Ok(Value::CurrentColor) => color_seen = true,
                    _ => return Err(cssparser::ParseError::custom(())),
                }
            } else {
                return Err(cssparser::ParseError::custom(()));
            }
        }
        if !(2..=4).contains(&lengths.len()) {
            return Err(cssparser::ParseError::custom(()));
        }
        let shadow = BoxShadow {
            offset_x: lengths[0],
            offset_y: lengths[1],
            blur: lengths.get(2).copied().unwrap_or(Length::Px(0.0)),
            spread: lengths.get(3).copied().unwrap_or(Length::Px(0.0)),
            color: shadow_color.unwrap_or(Color(0, 0, 0, 96)),
            current_color,
        };
        return Ok(vec![(P::BoxShadow, Value::BoxShadow(Some(shadow)))]);
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
                        _ => return Err(cssparser::ParseError::custom(())),
                    })
                })?;
            Value::Length(size)
        }
        P::FontWeight => {
            let weight = input.try_parse(|p| p.expect_number()).or_else(|_| {
                match keyword(input)?.as_str() {
                    "normal" => Ok(400.0),
                    "bold" => Ok(700.0),
                    _ => Err(cssparser::ParseError::custom(())),
                }
            })?;
            if !weight.is_finite() || !(1.0..=1000.0).contains(&weight) {
                return Err(cssparser::ParseError::custom(()));
            }
            Value::Number(weight)
        }
        P::Italic => Value::Bool(match keyword(input)?.as_str() {
            "normal" => false,
            "italic" | "oblique" => true,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Family => {
            let names = input.parse_comma_separated(|p| {
                let mut words = vec![];
                while let Ok(word) = p.try_parse(|p| p.expect_ident_or_string().cloned()) {
                    words.push(word.to_ascii_lowercase());
                }
                if words.is_empty() {
                    return Err(cssparser::ParseError::custom(()));
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
                        _ => return Err(cssparser::ParseError::custom(())),
                    }
                }
                if !underline && !strike {
                    return Err(cssparser::ParseError::custom(()));
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
                    return Err(cssparser::ParseError::custom(()));
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
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Align => Value::Align(match keyword(input)?.as_str() {
            "left" => TextAlign::Left,
            "right" => TextAlign::Right,
            "start" => TextAlign::Start,
            "end" => TextAlign::End,
            "center" => TextAlign::Center,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Direction => Value::Direction(match keyword(input)?.as_str() {
            "ltr" => Direction::Ltr,
            "rtl" => Direction::Rtl,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Display => Value::Display(match keyword(input)?.as_str() {
            "inline" => Display::Inline,
            "inline-block" => Display::InlineBlock,
            "block" | "list-item" => Display::Block,
            "flex" | "inline-flex" => Display::Flex,
            "grid" | "inline-grid" => Display::Grid,
            "none" => Display::None,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Float => Value::Float(match keyword(input)?.as_str() {
            "none" => FloatSide::None,
            "left" | "inline-start" => FloatSide::Left,
            "right" | "inline-end" => FloatSide::Right,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Position => Value::Position(match keyword(input)?.as_str() {
            "static" => Position::Static,
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Top | P::Right | P::Bottom | P::Left => Value::Length(length(input, true, true, true)?),
        P::GridColumns => Value::GridColumns(grid_column_count(input)?),
        P::Opacity => {
            let opacity = input.expect_number()?;
            if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                return Err(cssparser::ParseError::custom(()));
            }
            Value::Number(opacity)
        }
        P::FlexDirection => Value::FlexDirection(match keyword(input)?.as_str() {
            "row" => FlexDirection::Row,
            "column" => FlexDirection::Column,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::FlexWrap => Value::FlexWrap(match keyword(input)?.as_str() {
            "nowrap" => FlexWrap::NoWrap,
            "wrap" => FlexWrap::Wrap,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::JustifyContent => Value::JustifyContent(match keyword(input)?.as_str() {
            "flex-start" | "start" => JustifyContent::Start,
            "flex-end" | "end" => JustifyContent::End,
            "center" => JustifyContent::Center,
            "space-between" => JustifyContent::SpaceBetween,
            "space-around" => JustifyContent::SpaceAround,
            "space-evenly" => JustifyContent::SpaceEvenly,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::FlexGrow | P::FlexShrink => {
            let number = input.expect_number()?;
            if !number.is_finite() || number < 0.0 {
                return Err(cssparser::ParseError::custom(()));
            }
            Value::Number(number.min(1_000_000.0))
        }
        P::FlexBasis => Value::Length(length(input, true, false, true)?),
        P::RowGap | P::ColumnGap => Value::Length(length(input, false, false, true)?),
        P::ListMarker => Value::ListMarker(match keyword(input)?.as_str() {
            "none" => false,
            "disc" | "circle" | "square" | "decimal" => true,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::BorderWidth => Value::Length(border_width(input)?),
        P::BorderStyle => Value::Bool(border_style(input)?),
        P::BorderRadius => Value::Length(length(input, false, false, false)?),
        P::BoxShadow => unreachable!("box-shadow is parsed before scalar property values"),
        P::BoxSizing => Value::Bool(match keyword(input)?.as_str() {
            "content-box" => false,
            "border-box" => true,
            _ => return Err(cssparser::ParseError::custom(())),
        }),
        P::Width | P::MinWidth | P::MaxWidth | P::Height | P::MinHeight => {
            if p == P::MaxWidth && input.try_parse(|p| p.expect_ident_matching("none")).is_ok() {
                Value::Length(Length::Auto)
            } else {
                Value::Length(length(
                    input,
                    true,
                    false,
                    !matches!(p, P::Height | P::MinHeight),
                )?)
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

pub(super) fn supports_property(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "background"
            | "background-color"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-family"
            | "text-decoration"
            | "text-decoration-line"
            | "line-height"
            | "white-space"
            | "text-align"
            | "direction"
            | "display"
            | "float"
            | "position"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "grid-template-columns"
            | "opacity"
            | "flex-direction"
            | "flex-wrap"
            | "justify-content"
            | "flex-grow"
            | "flex-shrink"
            | "flex-basis"
            | "flex"
            | "gap"
            | "column-gap"
            | "row-gap"
            | "list-style"
            | "list-style-type"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "width"
            | "min-width"
            | "max-width"
            | "height"
            | "min-height"
            | "box-sizing"
            | "border"
            | "border-width"
            | "border-style"
            | "border-color"
            | "border-radius"
            | "box-shadow"
    )
}
fn border_width(input: &mut Parser<'_>) -> Result<Length, Error> {
    input
        .try_parse(|p| length(p, false, false, false))
        .or_else(|_| {
            Ok(Length::Px(match keyword(input)?.as_str() {
                "thin" => 1.0,
                "medium" => 3.0,
                "thick" => 5.0,
                _ => return Err(cssparser::ParseError::custom(())),
            }))
        })
}
fn border_style(input: &mut Parser<'_>) -> Result<bool, Error> {
    match keyword(input)?.as_str() {
        "solid" => Ok(true),
        "none" | "hidden" => Ok(false),
        _ => Err(cssparser::ParseError::custom(())),
    }
}

pub(super) fn apply(
    style: &mut ComputedStyle,
    parent: ComputedStyle,
    root: f32,
    viewport: (f32, f32),
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
            P::Direction => style.direction = source.direction,
            P::Display => style.display = source.display,
            P::Float => style.float_side = source.float_side,
            P::MarginTop | P::MarginRight | P::MarginBottom | P::MarginLeft => {
                let i = prop as usize - P::MarginTop as usize;
                style.margin[i] = source.margin[i];
            }
            P::PaddingTop | P::PaddingRight | P::PaddingBottom | P::PaddingLeft => {
                let i = prop as usize - P::PaddingTop as usize;
                style.padding[i] = source.padding[i];
            }
            P::Width => style.width = source.width,
            P::MinWidth => style.min_width = source.min_width,
            P::MaxWidth => style.max_width = source.max_width,
            P::Height => style.height = source.height,
            P::MinHeight => style.min_height = source.min_height,
            P::BoxSizing => style.border_box = source.border_box,
            P::BorderWidth => style.border_width = source.border_width,
            P::BorderColor => style.border_color = source.border_color,
            P::BorderStyle => style.border_solid = source.border_solid,
            P::BorderRadius => style.border_radius = source.border_radius,
            P::BoxShadow => style.box_shadow = source.box_shadow,
            P::FlexDirection => style.flex_direction = source.flex_direction,
            P::FlexWrap => style.flex_wrap = source.flex_wrap,
            P::JustifyContent => style.justify_content = source.justify_content,
            P::FlexGrow => style.flex_grow = source.flex_grow,
            P::FlexShrink => style.flex_shrink = source.flex_shrink,
            P::FlexBasis => style.flex_basis = source.flex_basis,
            P::RowGap => style.row_gap = source.row_gap,
            P::ColumnGap => style.column_gap = source.column_gap,
            P::Position => style.position = source.position,
            P::Top | P::Right | P::Bottom | P::Left => {
                let i = prop as usize - P::Top as usize;
                style.inset[i] = source.inset[i];
            }
            P::GridColumns => style.grid_columns = source.grid_columns,
            P::Opacity => style.opacity = source.opacity,
            P::ListMarker => style.list_marker = source.list_marker,
        }
        return;
    }
    let value = match value {
        Value::Length(value) => Value::Length(value.with_viewport(viewport.0, viewport.1)),
        Value::BoxShadow(shadow) => Value::BoxShadow(shadow.map(|mut shadow| {
            shadow.offset_x = shadow.offset_x.with_viewport(viewport.0, viewport.1);
            shadow.offset_y = shadow.offset_y.with_viewport(viewport.0, viewport.1);
            shadow.blur = shadow.blur.with_viewport(viewport.0, viewport.1);
            shadow.spread = shadow.spread.with_viewport(viewport.0, viewport.1);
            shadow
        })),
        value => value,
    };
    match (prop, value) {
        (P::FontSize, Value::Length(v)) => {
            style.font_size = v
                .resolve_with_viewport(
                    parent.font_size,
                    parent.font_size,
                    root,
                    viewport.0,
                    viewport.1,
                )
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
            style.line_height = LineHeight::Px(v.resolve_with_viewport(
                style.font_size,
                style.font_size,
                root,
                viewport.0,
                viewport.1,
            ))
        }
        (P::WhiteSpace, Value::WhiteSpace(v)) => style.white_space = v,
        (P::Align, Value::Align(v)) => style.text_align = v,
        (P::Direction, Value::Direction(v)) => style.direction = v,
        (P::Display, Value::Display(v)) => style.display = v,
        (P::Float, Value::Float(v)) => style.float_side = v,
        (P::FlexDirection, Value::FlexDirection(v)) => style.flex_direction = v,
        (P::FlexWrap, Value::FlexWrap(v)) => style.flex_wrap = v,
        (P::JustifyContent, Value::JustifyContent(v)) => style.justify_content = v,
        (P::FlexGrow, Value::Number(v)) => style.flex_grow = v,
        (P::FlexShrink, Value::Number(v)) => style.flex_shrink = v,
        (P::FlexBasis, Value::Length(v)) => style.flex_basis = v,
        (P::RowGap, Value::Length(v)) => style.row_gap = v,
        (P::ColumnGap, Value::Length(v)) => style.column_gap = v,
        (P::Position, Value::Position(v)) => style.position = v,
        (P::Top | P::Right | P::Bottom | P::Left, Value::Length(v)) => {
            style.inset[prop as usize - P::Top as usize] = v
        }
        (P::GridColumns, Value::GridColumns(v)) => style.grid_columns = v,
        (P::Opacity, Value::Number(v)) => style.opacity = (parent.opacity * v).clamp(0.0, 1.0),
        (P::ListMarker, Value::ListMarker(v)) => style.list_marker = v,
        (P::MarginTop | P::MarginRight | P::MarginBottom | P::MarginLeft, Value::Length(v)) => {
            style.margin[prop as usize - P::MarginTop as usize] = v
        }
        (P::PaddingTop | P::PaddingRight | P::PaddingBottom | P::PaddingLeft, Value::Length(v)) => {
            style.padding[prop as usize - P::PaddingTop as usize] = v
        }
        (P::Width, Value::Length(v)) => style.width = v,
        (P::MinWidth, Value::Length(v)) => style.min_width = v,
        (P::MaxWidth, Value::Length(v)) => style.max_width = v,
        (P::Height, Value::Length(v)) => style.height = v,
        (P::MinHeight, Value::Length(v)) => style.min_height = v,
        (P::BoxSizing, Value::Bool(v)) => style.border_box = v,
        (P::BorderWidth, Value::Length(v)) => style.border_width = v,
        (P::BorderRadius, Value::Length(v)) => style.border_radius = v,
        (P::BoxShadow, Value::BoxShadow(shadow)) => {
            style.box_shadow = shadow.map(|mut shadow| {
                if shadow.current_color {
                    shadow.color = style.color;
                }
                shadow
            });
        }
        (P::BorderStyle, Value::Bool(v)) => style.border_solid = v,
        _ => {}
    }
}
