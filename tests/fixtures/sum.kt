fun main() {
    val values = generateSequence(::readLine)
        .flatMap { it.trim().split(Regex("\\s+")).asSequence() }
        .filter { it.isNotEmpty() }
        .map(String::toLong)
        .toList()
    if (values.size != 2) throw IllegalArgumentException("expected two integers")
    println(values[0] + values[1])
}

