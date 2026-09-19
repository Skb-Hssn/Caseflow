import java.util.Scanner;

public final class Main {
    public static void main(String[] args) {
        Scanner input = new Scanner(System.in);
        if (!input.hasNextLong()) System.exit(2);
        long a = input.nextLong();
        if (!input.hasNextLong()) System.exit(2);
        long b = input.nextLong();
        System.out.println(a + b);
    }
}

