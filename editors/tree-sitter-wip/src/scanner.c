// Two tokens the grammar cannot say with a rule. The line break that ends
// a statement, a field, a variant or an arm:
//
// It is emitted only where the parser could end one there — which is never
// inside `( )` or `[ ]`, nor after an operator, a comma or an open bracket,
// so a line break there stays space — and never before `else`, which
// continues the line above. And a text block's text: all of
// it up to a `\\` or the closing `"""`, line breaks and quotes included.

#include "tree_sitter/parser.h"

#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

enum TokenType { TERMINATOR, ERROR_SENTINEL, TEXT_BLOCK_CONTENT };

// The one thing the scanner remembers: that it has given the empty
// terminator at the end of the file. The grammar repeats terminators, and
// an empty one could be repeated forever, so it is given once.
typedef struct {
    bool at_end;
} Scanner;

void *tree_sitter_wip_external_scanner_create(void) { return calloc(1, sizeof(Scanner)); }
void tree_sitter_wip_external_scanner_destroy(void *payload) { free(payload); }
unsigned tree_sitter_wip_external_scanner_serialize(void *payload, char *buffer) {
    buffer[0] = ((Scanner *)payload)->at_end;
    return 1;
}
void tree_sitter_wip_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
    ((Scanner *)payload)->at_end = length > 0 && buffer[0];
}

static void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

static bool is_word(int32_t c) {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_';
}

// A text block's text, up to a `\\` — an escape or `\\(` — or the closing
// `"""`; one or two quotes are text.
static bool text_block_content(TSLexer *lexer) {
    bool any = false;
    for (;;) {
        if (lexer->eof(lexer) || lexer->lookahead == '\\') {
            break;
        }
        if (lexer->lookahead == '"') {
            lexer->mark_end(lexer);
            int quotes = 0;
            while (lexer->lookahead == '"' && quotes < 3) {
                lexer->advance(lexer, false);
                quotes++;
            }
            if (quotes == 3) {
                // The end, which the grammar reads: the text is what came
                // before it.
                lexer->result_symbol = TEXT_BLOCK_CONTENT;
                return any;
            }
            any = true;
            continue;
        }
        lexer->advance(lexer, false);
        any = true;
    }
    lexer->mark_end(lexer);
    lexer->result_symbol = TEXT_BLOCK_CONTENT;
    return any;
}

bool tree_sitter_wip_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
    Scanner *scanner = payload;
    if (valid_symbols[TEXT_BLOCK_CONTENT] && !valid_symbols[ERROR_SENTINEL]) {
        return text_block_content(lexer);
    }
    // While recovering from an error every token is valid; a terminator
    // then, being empty at the end of the file, could be taken forever.
    if (valid_symbols[ERROR_SENTINEL] || !valid_symbols[TERMINATOR]) {
        return false;
    }
    lexer->result_symbol = TERMINATOR;
    // Space on this line first; a terminator needs a line break after it.
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t' || lexer->lookahead == '\r') {
        skip(lexer);
    }
    if (lexer->eof(lexer)) {
        if (scanner->at_end) {
            return false;
        }
        scanner->at_end = true;
        lexer->mark_end(lexer);
        return true;
    }
    scanner->at_end = false;
    // A comment that ends the line: the line break after it is the one.
    if (lexer->lookahead == '/') {
        return false;
    }
    if (lexer->lookahead != '\n') {
        return false;
    }
    skip(lexer);
    lexer->mark_end(lexer);
    // What begins the next line decides: `else` continues this one.
    for (;;) {
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t' || lexer->lookahead == '\r' ||
               lexer->lookahead == '\n') {
            skip(lexer);
        }
        if (lexer->lookahead != '/') {
            break;
        }
        // A comment on a line of its own: look past it.
        lexer->advance(lexer, true);
        if (lexer->lookahead != '/') {
            // `/` alone begins a line: an operator, which the parser reports.
            return true;
        }
        while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
            skip(lexer);
        }
    }
    const char *word = "else";
    for (size_t i = 0; i < strlen(word); i++) {
        if (lexer->lookahead != word[i]) {
            return true;
        }
        skip(lexer);
    }
    return is_word(lexer->lookahead);
}
