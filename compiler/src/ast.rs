use crate::lexer::Span;

#[derive(Debug, Clone)]
pub struct Module {
    pub name: Vec<String>,
    pub imports: Vec<Import>,
    pub declarations: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub path: Vec<String>,
    pub items: ImportItems,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ImportItems {
    All,
    Named(Vec<String>),
    Alias(String),
}

#[derive(Debug, Clone)]
pub enum Decl {
    Function(FnDecl),
    TypeDef(TypeDef),
    NewtypeDef(NewtypeDef),
    NominalDef(NominalDef),
    TraitDef(TraitDef),
    ImplBlock(ImplBlock),
    EffectDef(EffectDef),
    VibeDecl(VibeDecl),
}

#[derive(Debug, Clone)]
pub struct VibeDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub public: bool,
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub effects: Vec<TypeExpr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub type_ann: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeExpr {
    Named(String, Vec<TypeExpr>),
    Function(Vec<TypeExpr>, Box<TypeExpr>, Vec<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Record(Vec<(String, TypeExpr)>, Option<String>),
    Unit,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub public: bool,
    pub name: String,
    pub type_params: Vec<String>,
    pub body: TypeBody,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeBody {
    Record(Vec<(String, TypeExpr)>),
    Variants(Vec<Variant>),
    Alias(TypeExpr),
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<TypeExpr>,
}

#[derive(Debug, Clone)]
pub struct NewtypeDef {
    pub public: bool,
    pub name: String,
    pub type_params: Vec<String>,
    pub inner_type: TypeExpr,
    pub span: Span,
}

/// A nominal type definition: `nominal type Email = String`
/// Unlike type aliases, nominal types are NOT interchangeable with their inner type.
#[derive(Debug, Clone)]
pub struct NominalDef {
    pub public: bool,
    pub name: String,
    pub type_params: Vec<String>,
    pub inner_type: TypeExpr,
    pub span: Span,
}

/// A type parameter with optional trait bounds: `A: Eq + Ord`
#[derive(Debug, Clone)]
pub struct TypeParamBound {
    pub name: String,
    pub bounds: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TraitDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub requires: Vec<TypeExpr>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub trait_name: String,
    pub type_params: Vec<String>,
    pub target: TypeExpr,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EffectDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub operations: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Expr {
    // Literals
    IntLit(i64, Span),
    FloatLit(f64, Span),
    StringLit(String, Span),
    StringInterp(Vec<StringPart>, Span), // "hello ${name}, age ${age}"
    CharLit(char, Span),
    BoolLit(bool, Span),
    UnitLit(Span),

    // Identifiers
    Ident(String, Span),
    TypeConstructor(String, Span),

    // Compound
    List(Vec<Expr>, Span),
    Tuple(Vec<Expr>, Span),
    Record(Vec<(String, Expr)>, Span),
    RecordUpdate(Box<Expr>, Vec<(String, Expr)>, Span),
    FieldAccess(Box<Expr>, String, Span),

    // Operations
    BinOp(Box<Expr>, BinOp, Box<Expr>, Span),
    UnaryOp(UnaryOp, Box<Expr>, Span),
    Pipe(Box<Expr>, Box<Expr>, Span),

    // Function related
    Call(Box<Expr>, Vec<Expr>, Span),
    Lambda(Vec<Param>, Box<Expr>, Span),

    // Control flow
    If(Box<Expr>, Box<Expr>, Option<Box<Expr>>, Span),
    Match(Box<Expr>, Vec<MatchArm>, Span),
    When(Vec<WhenClause>, Span), // when { cond -> expr, ... }
    DoBlock(Vec<Expr>, Span),

    // Bindings
    Let(Pattern, Option<TypeExpr>, Box<Expr>, Box<Expr>, Span),
    LetBind(Pattern, Option<TypeExpr>, Box<Expr>, Span),
    LetElse(Pattern, Option<TypeExpr>, Box<Expr>, Box<Expr>, Span), // let pat = expr else fallback

    // Effects
    Handle(Box<Expr>, Vec<Handler>, Span),
    Resume(Box<Expr>, Span),
    Perform(String, String, Vec<Expr>, Span), // effect_name, operation, args

    // Concurrency
    Par(Vec<Expr>, Span),                        // par(expr1, expr2, ...)
    Pmap(Box<Expr>, Box<Expr>, Span),            // pmap(collection, function)
    Pfilter(Box<Expr>, Box<Expr>, Span),         // pfilter(collection, predicate)
    Preduce(Box<Expr>, Box<Expr>, Box<Expr>, Span), // preduce(collection, init, function)
    Race(Vec<Expr>, Span),                       // race(expr1, expr2, ...)
    ChanCreate(Box<Expr>, Span),                 // create_channel(capacity)
    ChanSend(Box<Expr>, Box<Expr>, Span),        // send(channel, value)
    ChanRecv(Box<Expr>, Span),                   // recv(channel)
    VibePipeline(Box<Expr>, Vec<PipelineStage>, Span), // vibe: source |> stages |> terminal

    // Actors
    SpawnActor(Box<Expr>, Span),                 // spawn(handler)
    SendTo(Box<Expr>, Box<Expr>, Span),          // send_to(actor, message)
    WithTimeout(Box<Expr>, Box<Expr>, Span),     // with_timeout(duration, expr)
}

#[derive(Debug, Clone)]
pub enum StringPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum PipelineStage {
    Map(Expr),
    Filter(Expr),
    FlatMap(Expr),
    FilterMap(Expr),
    Take(Expr),
    Drop(Expr),
    TakeWhile(Expr),
    DropWhile(Expr),
    Fold(Expr, Expr),     // initial, function
    ForEach(Expr),
    Collect,
    Count,
    First,
    Last,
    Scan(Expr, Expr),     // initial, function
    SortBy(Expr),
    Distinct,
    GroupBy(Expr),
    Chunk(Expr),
    Any(Expr),
    All(Expr),
    Reduce(Expr),
    Inspect(Expr),
}

#[derive(Debug, Clone)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Concat,
    Compose, // >> function composition
}

#[derive(Debug, Clone)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard(Span),
    Ident(String, Span),
    IntLit(i64, Span),
    FloatLit(f64, Span),
    StringLit(String, Span),
    BoolLit(bool, Span),
    CharLit(char, Span),
    Constructor(String, Vec<Pattern>, Span),
    Tuple(Vec<Pattern>, Span),
    Record(Vec<(String, Pattern)>, Span),
}

#[derive(Debug, Clone)]
pub struct WhenClause {
    pub condition: Expr,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub struct Handler {
    pub effect_name: String,
    pub operation: String,
    pub params: Vec<String>,
    pub body: Expr,
}
