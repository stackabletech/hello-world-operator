package tech.stackable.helloworld;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class HelloController {

	private final Logger log = LoggerFactory.getLogger(this.getClass());

	@Value("${greeting.recipient:World}")
	String recipient;

	@Value("${greeting.color:black}")
	String color;

	@GetMapping("/")
	public String index() {
		log.info("Received an HTTP request.");
		return "<h1 style=\"color:" + color +"\">Hello " + recipient + "!</h1>";
	}
}