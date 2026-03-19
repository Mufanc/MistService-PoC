package mist;

interface IMistService {
    int[] idmapList() = 1;
    boolean idmapGet(int id) = 2;
    void idmapSet(int id, boolean value) = 3;
    void idmapClear() = 4;
}
