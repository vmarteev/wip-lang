#ifndef VECTORS_H
#define VECTORS_H

/* A library's own vectors, which a binding declares as Wip's, laid out
   alike: a value crosses as it is. */
typedef struct Vector2 {
    float x;
    float y;
} Vector2;

typedef struct Vector3 {
    float x;
    float y;
    float z;
} Vector3;

/* Four floats, aligned as floats are: not Wip's `Vec4`, which a GPU's
   `vec4` aligns to 16. */
typedef struct Vector4 {
    float x;
    float y;
    float z;
    float w;
} Vector4;

typedef struct Mesh {
    Vector3 center;
    Vector2 size;
    int count;
} Mesh;

Vector3 vector_add(Vector3 a, Vector3 b);
void vector_scale(Vector3 *v, float by);
float mesh_area(const Mesh *mesh);

#endif
