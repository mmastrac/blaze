#include <emscripten.h>
#include <stdio.h>
#include "game.h"

extern const char* termenv;

void wait_display(void) {
}

int read_scores(int *scores, int max_scores) {
    // TODO: save/load scores
    return 0;
}

int save_score(int score) {
    // TODO: save/load scores
    return 0;
}

EMSCRIPTEN_KEEPALIVE int _init_game(void) {
    termenv = "vt420";
    term_width = 80;
    return init_game();
}

EMSCRIPTEN_KEEPALIVE int _update(int msec) {
    return update(msec);
}

EMSCRIPTEN_KEEPALIVE void _game_input(int input) {
    game_input(input);
}
