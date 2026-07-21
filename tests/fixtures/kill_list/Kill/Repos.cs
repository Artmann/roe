namespace Kill;

public interface IRepo
{
    void Save();
}

public class SqlRepo : IRepo
{
    public void Save()
    {
    }
}
