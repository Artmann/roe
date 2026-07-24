namespace HealthSuppress;

public class Widget
{
    // Trips both high-complexity and high-cognitive-complexity, but only the
    // first is suppressed — the other must still be reported.
    // roe-ignore-next-line high-complexity
    public void Scoped(bool a, bool b, bool c)
    {
        if (a)
        {
            DoA();
        }
        else if (b && c)
        {
            DoB();
        }
    }

    // A bare marker takes out every check on the line below it.
    // roe-ignore-next-line
    public void Bare(bool a, bool b, bool c)
    {
        if (a)
        {
            DoA();
        }
        else if (b && c)
        {
            DoB();
        }
    }

    public void Unmarked(bool a, bool b, bool c)
    {
        if (a)
        {
            DoA();
        }
        else if (b && c)
        {
            DoB();
        }
    }

    private void DoA() { }

    private void DoB() { }
}
