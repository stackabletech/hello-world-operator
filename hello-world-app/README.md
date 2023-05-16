# Hello World App

This is a demo application that the operator starts up

## Building & Running

Requires Java 17. Check with `./mvnw -version` and set `JAVA_HOME` accordingly.

To run: `./mvnw spring-boot:run` or `make run`

To build: `make package`

The built file is found at `./target/hello-world-0.0.1-SNAPSHOT.jar`

## Configuration

The config file should be called `application.properties` and should be located next to the jar, or in a `config` directory or on the classpath (this is the Spring Boot default). You can configure any Spring Boot properties (such as `server.port`) as well as:

- `greeting.recipient` (String)
- `greeting.color` (String)
