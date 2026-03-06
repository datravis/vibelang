# Appendix A: Formal Grammar

This appendix provides the formal grammar of VibeLang in extended BNF notation.

## A.1 Top-Level

```ebnf
program        = module_decl, { import_decl }, { top_decl } ;
module_decl    = "module", module_path ;
module_path    = IDENT, { ".", IDENT } ;

import_decl    = "use", module_path, [ import_spec ] ;
import_spec    = ".", "{", IDENT, { ",", IDENT }, "}"
               | ".", "*"
               | "as", IDENT ;

top_decl       = fn_decl
               | type_decl
               | trait_decl
               | impl_decl
               | effect_decl
               | newtype_decl
               | alias_decl ;
```

## A.2 Type Declarations

```ebnf
type_decl      = "type", TYPE_IDENT, [ type_params ], "=", type_body,
                 [ "deriving", trait_list ] ;
type_body      = record_type | variant_type ;
record_type    = "{", field_decl, { ",", field_decl }, [ "," ], "}" ;
field_decl     = IDENT, ":", type_expr ;
variant_type   = "|", variant, { "|", variant } ;
variant        = TYPE_IDENT, [ "(", type_expr, { ",", type_expr }, ")" ] ;

newtype_decl   = "newtype", TYPE_IDENT, "=", type_expr ;
alias_decl     = "type", "alias", TYPE_IDENT, [ type_params ], "=", type_expr ;
```

## A.3 Function Declarations

```ebnf
fn_decl        = [ "pub" ], [ "unsafe" ], "fn", IDENT, [ type_params ],
                 "(", [ param_list ], ")", "->", type_expr,
                 [ effect_clause ], "=", expr ;

param_list     = param, { ",", param } ;
param          = IDENT, ":", [ ownership ], type_expr ;
ownership      = "own" | "ref" | "share" ;

effect_clause  = "with", effect_ref, { ",", effect_ref } ;
effect_ref     = TYPE_IDENT, [ "[", type_expr, { ",", type_expr }, "]" ] ;
```

## A.4 Trait and Effect Declarations

```ebnf
trait_decl     = "trait", TYPE_IDENT, [ type_params ],
                 [ "requires", trait_ref, { ",", trait_ref } ],
                 "{", { fn_sig }, "}" ;

impl_decl      = "impl", trait_ref, "{", { fn_decl }, "}" ;

trait_ref      = TYPE_IDENT, [ "[", type_expr, { ",", type_expr }, "]" ] ;

effect_decl    = "effect", TYPE_IDENT, [ type_params ],
                 "{", { fn_sig }, "}" ;

fn_sig         = "fn", IDENT, [ type_params ],
                 "(", [ param_list ], ")", "->", type_expr ;
```

## A.5 Expressions

```ebnf
expr           = let_expr
               | if_expr
               | match_expr
               | when_expr
               | do_block
               | handle_expr
               | lambda_expr
               | pipe_expr ;

let_expr       = "let", pattern, [ ":", type_expr ], "=", expr,
                 [ "else", expr ], [ "in", expr ] ;

if_expr        = "if", expr, "then", expr, "else", expr ;

match_expr     = "match", expr, { match_arm } ;
match_arm      = "|", pattern, [ "when", expr ], "->", expr ;

when_expr      = "when", { when_arm } ;
when_arm       = "|", expr, "->", expr
               | "|", "otherwise", "->", expr ;

do_block       = "do", { stmt }, expr ;
stmt           = let_expr | expr ;

handle_expr    = "handle", expr, "with", effect_ref,
                 "{", { handler_arm }, "}" ;
handler_arm    = IDENT, "(", [ param_list ], ")", "->", expr ;

lambda_expr    = "fn", "(", [ param_list ], ")", [ "->", type_expr ], "=", expr
               | "\\", IDENT, { ",", IDENT }, "->", expr ;

pipe_expr      = unary_expr, { "|>", unary_expr } ;
```

## A.6 Patterns

```ebnf
pattern        = "_"                                    -- wildcard
               | IDENT                                  -- variable binding
               | literal                                -- literal match
               | TYPE_IDENT, [ "(", pattern_list, ")" ] -- variant destructure
               | "{", field_pattern_list, "}"           -- record destructure
               | "(", pattern_list, ")"                 -- tuple destructure
               | pattern, "::", type_expr ;             -- type-annotated pattern

pattern_list   = pattern, { ",", pattern } ;
field_pattern_list = field_pattern, { ",", field_pattern } ;
field_pattern  = IDENT, [ ":", pattern ] ;
```

## A.7 Type Expressions

```ebnf
type_expr      = TYPE_IDENT, [ "[", type_expr, { ",", type_expr }, "]" ]
               | "fn", "(", [ type_list ], ")", "->", type_expr, [ effect_clause ]
               | "(", type_expr, { ",", type_expr }, ")"
               | "{", field_decl, { ",", field_decl }, [ "|", IDENT ], "}" ;

type_list      = type_expr, { ",", type_expr } ;
type_params    = "[", type_param, { ",", type_param }, "]" ;
type_param     = TYPE_IDENT, [ ":", trait_bound ] ;
trait_bound    = trait_ref, { "+", trait_ref } ;
```

## A.8 Operator Precedence (Highest to Lowest)

| Precedence | Operators | Associativity |
|------------|-----------|---------------|
| 10 | function application | left |
| 9 | `!`, `~`, unary `-` | prefix |
| 8 | `*`, `/`, `%` | left |
| 7 | `+`, `-` | left |
| 6 | `++` | right |
| 5 | `<<`, `>>` | left |
| 4 | `&` | left |
| 3 | `^` | left |
| 2 | `\|` (bitwise) | left |
| 1 | `==`, `!=`, `<`, `>`, `<=`, `>=` | none (no chaining) |
| 0 | `&&` | left |
| -1 | `\|\|` | left |
| -2 | `\|>` | left |
| -3 | `>>` (compose) | right |
| -4 | `::` (type annotation) | left |

## A.9 Lexical Rules

```ebnf
IDENT          = lower, { lower | upper | digit | "_" } ;
TYPE_IDENT     = upper, { lower | upper | digit } ;

literal        = int_lit | float_lit | string_lit | char_lit | bool_lit | unit_lit ;
int_lit        = digit, { digit | "_" }
               | "0x", hex_digit, { hex_digit | "_" }
               | "0b", bin_digit, { bin_digit | "_" }
               | "0o", oct_digit, { oct_digit | "_" } ;
float_lit      = digit, { digit }, ".", digit, { digit }, [ exponent ] ;
exponent       = ("e" | "E"), [ "+" | "-" ], digit, { digit } ;
string_lit     = '"', { string_char | escape | interpolation }, '"' ;
interpolation  = "${", expr, "}" ;
char_lit       = "'", ( char | escape ), "'" ;
bool_lit       = "true" | "false" ;
unit_lit       = "(", ")" ;
```

## A.10 Reserved Keywords

```
and       as        do        effect    else      false     fn
for       handle    if        impl      in        let       match
module    newtype   not       of        or        otherwise par
pmap      pub       ref       region    resume    return    share
spawn     stream    test      then      trait     true      type
unsafe    use       when      with
```

## A.11 File Extension

VibeLang source files use the `.vibe` extension.
Module interface files use the `.vibei` extension.
