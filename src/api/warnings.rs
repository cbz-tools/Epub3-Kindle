use std::{fmt, ops::Index};

/// A warning category emitted when conversion completes with a known semantic
/// limitation.
///
/// The codes map directly to the unsupported-feature handling requirements in
/// the conversion audit: `W001` ↔ `UFH-001`, `W002` ↔ `UFH-002`, `W003` ↔
/// `UFH-003`, `W004` ↔ `UFH-004`, `W005` ↔ `UFH-005`, and `W006` ↔ `UFH-006`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningCode {
    /// `W001` / `UFH-001`: interactive or executable semantics were degraded.
    W001,
    /// `W002` / `UFH-002`: a rich or alternate representation used a fallback.
    W002,
    /// `W003` / `UFH-003`: timed or media semantics were degraded.
    W003,
    /// `W004` / `UFH-004`: presentation semantics were degraded.
    W004,
    /// `W005` / `UFH-005`: structured semantics were reduced to readable content.
    W005,
    /// `W006` / `UFH-006`: publishing guidance was exceeded safely.
    W006,
}

impl fmt::Display for WarningCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::W001 => "W001",
            Self::W002 => "W002",
            Self::W003 => "W003",
            Self::W004 => "W004",
            Self::W005 => "W005",
            Self::W006 => "W006",
        };
        formatter.write_str(code)
    }
}

/// A warning produced during an otherwise successful conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionWarning {
    /// The category of the warning.
    pub code: WarningCode,
    /// A human-readable explanation of the warning.
    pub message: String,
}

impl ConversionWarning {
    /// Creates a warning from a code and a human-readable message.
    pub fn new(code: WarningCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Collects conversion warnings while preserving their insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WarningCollector {
    warnings: Vec<ConversionWarning>,
}

impl WarningCollector {
    /// Creates an empty warning collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one warning to the end of the collection.
    pub fn push(&mut self, warning: ConversionWarning) {
        self.warnings.push(warning);
    }

    /// Adds a warning to the end of the collection from its components.
    pub fn add(&mut self, code: WarningCode, message: impl Into<String>) {
        self.push(ConversionWarning::new(code, message));
    }

    /// Adds a warning unless the same code and message are already present.
    pub fn add_once(&mut self, code: WarningCode, message: impl Into<String>) {
        let message = message.into();
        if !self
            .warnings
            .iter()
            .any(|warning| warning.code == code && warning.message == message)
        {
            self.add(code, message);
        }
    }

    /// Adds one warning for a category, ignoring later warnings in that category.
    pub fn add_category_once(&mut self, code: WarningCode, message: impl Into<String>) {
        if !self.warnings.iter().any(|warning| warning.code == code) {
            self.add(code, message);
        }
    }

    /// Adds all warnings from `warnings` in iteration order.
    pub fn extend<I>(&mut self, warnings: I)
    where
        I: IntoIterator<Item = ConversionWarning>,
    {
        self.warnings.extend(warnings);
    }

    /// Returns the number of collected warnings.
    pub fn len(&self) -> usize {
        self.warnings.len()
    }

    /// Returns whether no warnings have been collected.
    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Returns the collected warnings in insertion order.
    pub fn as_slice(&self) -> &[ConversionWarning] {
        &self.warnings
    }

    /// Returns the collected warnings, preserving insertion order.
    pub fn into_warnings(self) -> Vec<ConversionWarning> {
        self.warnings
    }
}

impl Index<usize> for WarningCollector {
    type Output = ConversionWarning;

    fn index(&self, index: usize) -> &Self::Output {
        &self.warnings[index]
    }
}

impl IntoIterator for WarningCollector {
    type Item = ConversionWarning;
    type IntoIter = std::vec::IntoIter<ConversionWarning>;

    fn into_iter(self) -> Self::IntoIter {
        self.warnings.into_iter()
    }
}

impl<'a> IntoIterator for &'a WarningCollector {
    type Item = &'a ConversionWarning;
    type IntoIter = std::slice::Iter<'a, ConversionWarning>;

    fn into_iter(self) -> Self::IntoIter {
        self.warnings.iter()
    }
}

/// The result of a successful conversion and any warnings it produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionOutcome<T> {
    /// The converted value.
    pub value: T,
    /// Warnings collected during the successful conversion, in insertion order.
    pub warnings: WarningCollector,
}

impl<T> ConversionOutcome<T> {
    /// Creates a successful outcome with no warnings.
    pub fn new(value: T) -> Self {
        Self {
            value,
            warnings: WarningCollector::new(),
        }
    }

    /// Creates a successful outcome from an existing warning collector.
    pub fn from_collector(value: T, warnings: WarningCollector) -> Self {
        Self { value, warnings }
    }

    /// Returns a successful outcome with one additional warning.
    pub fn with_warning(mut self, code: WarningCode, message: impl Into<String>) -> Self {
        self.warnings.add(code, message);
        self
    }

    /// Returns a successful outcome with warnings appended in iterator order.
    pub fn with_warnings<I, M>(mut self, warnings: I) -> Self
    where
        I: IntoIterator<Item = (WarningCode, M)>,
        M: Into<String>,
    {
        for (code, message) in warnings {
            self.warnings.add(code, message);
        }
        self
    }

    /// Returns a reference to the converted value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Returns a mutable reference to the converted value.
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Returns the public warning collection in insertion order.
    pub fn warnings(&self) -> &[ConversionWarning] {
        self.warnings.as_slice()
    }

    /// Returns whether the outcome contains one or more warnings.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Consumes the outcome and returns the converted value.
    pub fn into_value(self) -> T {
        self.value
    }

    /// Consumes the outcome and returns the value and warnings separately.
    pub fn into_parts(self) -> (T, WarningCollector) {
        (self.value, self.warnings)
    }
}
