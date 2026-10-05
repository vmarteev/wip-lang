#include <stdlib.h>
#include <string.h>

typedef struct Db { long long rows; } Db;

/* An out-parameter, as `sqlite3_open` has: the handle is written through a
   pointer the caller supplies. */
int db_open(const char *name, Db **out) {
    if (strcmp(name, "bad") == 0) {
        *out = NULL;
        return 1;
    }
    Db *db = malloc(sizeof(Db));
    db->rows = (long long) strlen(name);
    *out = db;
    return 0;
}

long long db_rows(const Db *db) { return db->rows; }
void db_close(Db *db) { free(db); }
void *db_as_void(Db *db) { return (void *) db; }
