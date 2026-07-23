namespace Lib;

// Consumed only by an out-of-solution Unity project that references the
// built DLL directly — nothing inside this workspace ever calls it.
public static class PublicApi
{
    public static int ComputeSomethingUsedByUnity(int input)
    {
        return input * 2;
    }
}
